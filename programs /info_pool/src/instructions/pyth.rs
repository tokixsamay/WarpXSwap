use anchor_lang::prelude::*;
use pyth_solana_receiver_sdk::price_update::{PriceUpdateV2, get_feed_id_from_hex};
use crate::state::*;
use crate::constants::*;
use crate::errors::InfoPoolError;
use crate::utils::*;

use pool_program::cpi as pool_cpi;
use pool_program::cpi::accounts::UpdateOraclePrice;

// ═══════════════════════════════════════════════════
// UPDATE PYTH FEEDS
// Called by off-chain crank every ~400ms (each block)
// Reads Pyth price, confidence, then updates per-asset
// EMAs for short / medium / long timeframes.
//
// TWAP approach: time-weighted EMA
//   alpha = dt_slots / period_slots  (capped at 1)
//   new_ema = old_ema + alpha × (spot − old_ema)
//
// This gives proper 30-min / 4-hr / 24-hr trailing
// averages without a fixed-size ring buffer.
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
#[instruction(mint: Pubkey)]
pub struct UpdatePythFeeds<'info> {
    #[account(
        mut,
        seeds = [INFO_POOL_SEED, info_pool.pool_id.as_ref()],
        bump = info_pool.bump,
    )]
    pub info_pool: Account<'info, InfoPoolAccount>,

    /// Pyth price account for this asset
    pub price_update: Account<'info, PriceUpdateV2>,

    /// Crank authority — must be the InfoPool's registered authority.
    /// Prevents arbitrary signers from manipulating price feeds.
    #[account(
        constraint = crank.key() == info_pool.authority @ InfoPoolError::NotCrank
    )]
    pub crank: Signer<'info>,
}

pub fn handler_update_feeds(
    ctx: Context<UpdatePythFeeds>,
    mint: Pubkey,
) -> Result<()> {
    let info_pool    = &mut ctx.accounts.info_pool;
    let price_update = &ctx.accounts.price_update;
    let clock        = Clock::get()?;

    // ── FIND ASSET IN INFO POOL ───────────────────
    // Must find before price read so we can use per-asset feed ID.
    let asset = info_pool.assets
        .iter_mut()
        .find(|a| a.mint == mint)
        .ok_or(InfoPoolError::AssetNotFound)?;

    // ── GUARD: feed ID must be configured ─────────
    // Zeros means governance_set_pyth_feed_id was never called.
    require!(
        asset.pyth_feed_id != [0u8; 32],
        InfoPoolError::PythFeedNotConfigured
    );

    // ── READ PYTH PRICE (per-asset feed ID) ───────
    // get_price_no_older_than validates the feed ID AND staleness
    // (PYTH_MAX_STALENESS seconds).  No separate stale check needed.
    let price_feed = price_update.get_price_no_older_than(
        &clock,
        PYTH_MAX_STALENESS as u64,
        &asset.pyth_feed_id,
    )?;

    let current_price = price_feed.price;
    let confidence    = price_feed.conf;

    // ── COMPUTE TIME DELTA (slots since last update) ──
    // clock.slot gives actual slot number; constants TWAP_*_SLOTS are in slots.
    let current_slot = clock.slot as i64;
    let dt_slots: u64 = if asset.pyth_data.last_updated > 0 {
        (current_slot - asset.pyth_data.last_updated).max(0) as u64
    } else {
        // First reading — initialise EMAs to current price
        0
    };

    // ── UPDATE EMAs (time-weighted, no ring buffer) ──
    // Short  (~30 min):  alpha = dt / TWAP_SHORT_SLOTS
    // Medium (~4 hr):    alpha = dt / TWAP_MEDIUM_SLOTS
    // Long   (~24 hr):   alpha = dt / TWAP_LONG_SLOTS
    asset.pyth_data.twap_short = ema_update(
        asset.pyth_data.twap_short,
        current_price,
        dt_slots,
        TWAP_SHORT_SLOTS,
    );
    asset.pyth_data.twap_medium = ema_update(
        asset.pyth_data.twap_medium,
        current_price,
        dt_slots,
        TWAP_MEDIUM_SLOTS,
    );
    asset.pyth_data.twap_long = ema_update(
        asset.pyth_data.twap_long,
        current_price,
        dt_slots,
        TWAP_LONG_SLOTS,
    );

    // Update spot price and metadata
    asset.pyth_data.price        = current_price;
    asset.pyth_data.confidence   = confidence;
    asset.pyth_data.last_updated = current_slot;

    emit!(PythFeedsUpdated {
        pool_id:    info_pool.pool_id,
        mint,
        price:      current_price,
        confidence,
        twap_short: asset.pyth_data.twap_short,
        twap_medium: asset.pyth_data.twap_medium,
        twap_long:  asset.pyth_data.twap_long,
        slot:       current_slot,
    });

    Ok(())
}

// ═══════════════════════════════════════════════════
// PUSH VOLUME
// Called by the off-chain crank after fetching 24h trading
// volume from an external source (e.g. DexScreener).
//
// Volume is separate from Pyth price data — it has a much
// lower refresh cadence (once per minute is sufficient) and
// comes from a different API.  Splitting it out keeps
// update_pyth_feeds focused on price-feed ticks.
//
// On each call:
//   1. Shift volume_24h → volume_prev  (previous window)
//   2. Write the new volume_24h value
//
// The 3-layer check (check_volume_layer) then compares
// volume_24h vs volume_prev to decide if volume is
// consistently rising (≥10%).
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
#[instruction(mint: Pubkey)]
pub struct PushVolume<'info> {
    #[account(
        mut,
        seeds = [INFO_POOL_SEED, info_pool.pool_id.as_ref()],
        bump = info_pool.bump,
    )]
    pub info_pool: Account<'info, InfoPoolAccount>,

    /// Crank authority — must be the InfoPool's registered authority.
    #[account(
        constraint = crank.key() == info_pool.authority @ InfoPoolError::NotCrank
    )]
    pub crank: Signer<'info>,
}

pub fn handler_push_volume(
    ctx: Context<PushVolume>,
    mint:       Pubkey,
    volume_24h: u64,
) -> Result<()> {
    let info_pool = &mut ctx.accounts.info_pool;

    let asset = info_pool.assets
        .iter_mut()
        .find(|a| a.mint == mint)
        .ok_or(InfoPoolError::AssetNotFound)?;

    // Bug #9 fix: rotate 3-period history before writing the new value.
    // volume_history[0] = oldest (3 periods ago)
    // volume_history[1] = 2 periods ago
    // volume_history[2] = 1 period ago (the period just completed)
    // On each push: shift history forward, storing the PREVIOUS volume_24h
    // into history[2], then write the new value into volume_24h.
    //
    // Old behaviour stored only one previous value (volume_prev), which could
    // be satisfied by a single high-volume candle.  check_volume_layer now
    // requires all 3 pairwise transitions to show ≥10% growth.
    asset.pyth_data.volume_history[0] = asset.pyth_data.volume_history[1];
    asset.pyth_data.volume_history[1] = asset.pyth_data.volume_history[2];
    asset.pyth_data.volume_history[2] = asset.pyth_data.volume_24h;
    asset.pyth_data.volume_24h        = volume_24h;

    emit!(VolumeUpdated {
        pool_id:    info_pool.pool_id,
        mint,
        volume_24h,
        volume_h0:  asset.pyth_data.volume_history[0],
        volume_h1:  asset.pyth_data.volume_history[1],
        volume_h2:  asset.pyth_data.volume_history[2],
    });

    Ok(())
}

// ═══════════════════════════════════════════════════
// PUSH ORACLE PRICE TO POOL
// Called by the crank after each update_pyth_feeds tick.
// Pushes the freshly-read Pyth spot price into Pool's
// AssetAccount.oracle_price via CPI, so Pool's swap can
// use oracle rates without a circular CPI dependency
// (Pool → InfoPool → Pool would be circular at the Rust
// crate level since InfoPool already depends on pool-program).
//
// Crank sequence (per asset, per tick):
//   1. update_pyth_feeds       → update InfoPool prices + EMAs
//   2. push_oracle_price_to_pool → sync price into Pool AssetAccount
//   3. run_threshold_check     → 3-layer engine, may block inflow
//   4. calculate_and_push_fee  → V-shape fee → push to Pool
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
#[instruction(mint: Pubkey)]
pub struct PushOraclePriceToPool<'info> {
    #[account(
        seeds = [INFO_POOL_SEED, info_pool.pool_id.as_ref()],
        bump = info_pool.bump,
    )]
    pub info_pool: Account<'info, InfoPoolAccount>,

    /// Pool Program — key verified in handler
    /// CHECK: Program ID checked against POOL_PROGRAM_ID constant before CPI
    pub pool_program: AccountInfo<'info>,

    /// Pool PDA in Pool program
    /// CHECK: Validated by Pool program's PDA constraints
    #[account(mut)]
    pub pool_account: AccountInfo<'info>,

    /// Asset PDA in Pool program (mut — oracle_price written here)
    /// CHECK: Validated by Pool program's PDA constraints
    #[account(mut)]
    pub asset_account: AccountInfo<'info>,

    /// Crank authority — must be the InfoPool's registered authority.
    #[account(
        constraint = crank.key() == info_pool.authority @ InfoPoolError::NotCrank
    )]
    pub crank: Signer<'info>,
}

pub fn handler_push_oracle_price(
    ctx: Context<PushOraclePriceToPool>,
    mint: Pubkey,
) -> Result<()> {
    // ── VERIFY POOL PROGRAM IDENTITY ──────────────
    require!(
        ctx.accounts.pool_program.key().to_string() == POOL_PROGRAM_ID,
        InfoPoolError::NotPoolProgram
    );

    // ── READ CURRENT PRICE FROM INFO POOL ─────────
    let price_raw = {
        let info_pool = &ctx.accounts.info_pool;
        let asset = info_pool.assets
            .iter()
            .find(|a| a.mint == mint)
            .ok_or(InfoPoolError::AssetNotFound)?;
        asset.pyth_data.price
    };

    // Only push positive prices — negative/zero Pyth prices mean data error
    require!(price_raw > 0, InfoPoolError::InvalidOraclePrice);
    // Safe conversion: try_into() catches the impossible negative-after-check
    // case, avoiding silent wrapping that i64 as u64 would allow.
    let price: u64 = price_raw
        .try_into()
        .map_err(|_| error!(InfoPoolError::InvalidOraclePrice))?;

    // ── CPI: PUSH PRICE TO POOL ────────────────────
    let pool_id_bytes = ctx.accounts.info_pool.pool_id.to_bytes();
    let bump          = ctx.accounts.info_pool.bump;
    let ip_seeds: &[&[u8]] = &[INFO_POOL_SEED, &pool_id_bytes, &[bump]];
    let signer_seeds  = &[ip_seeds];

    pool_cpi::update_oracle_price(
        CpiContext::new_with_signer(
            ctx.accounts.pool_program.to_account_info(),
            UpdateOraclePrice {
                pool:                ctx.accounts.pool_account.to_account_info(),
                asset:               ctx.accounts.asset_account.to_account_info(),
                info_pool_authority: ctx.accounts.info_pool.to_account_info(),
            },
            signer_seeds,
        ),
        mint,
        price,
    )?;

    emit!(OraclePricePushed {
        pool_id: ctx.accounts.info_pool.pool_id,
        mint,
        price,
    });

    Ok(())
}

// ── EVENTS ────────────────────────────────────────
#[event]
pub struct OraclePricePushed {
    pub pool_id: Pubkey,
    pub mint:    Pubkey,
    pub price:   u64,
}

#[event]
pub struct VolumeUpdated {
    pub pool_id:    Pubkey,
    pub mint:       Pubkey,
    pub volume_24h: u64,
    /// Most recent completed period (1 period ago after rotation)
    pub volume_h2:  u64,
    /// 2 periods ago
    pub volume_h1:  u64,
    /// 3 periods ago (oldest)
    pub volume_h0:  u64,
}

#[event]
pub struct PythFeedsUpdated {
    pub pool_id:     Pubkey,
    pub mint:        Pubkey,
    pub price:       i64,
    pub confidence:  u64,
    pub twap_short:  i64,
    pub twap_medium: i64,
    pub twap_long:   i64,
    pub slot:        i64,
      }
      
