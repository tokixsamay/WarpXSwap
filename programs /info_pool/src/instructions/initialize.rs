use anchor_lang::prelude::*;
use crate::state::*;
use crate::constants::*;
use crate::errors::InfoPoolError;

// ═══════════════════════════════════════════════════
// INITIALIZE INFO POOL
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
#[instruction(pool_id: Pubkey)]
pub struct InitializeInfoPool<'info> {
    #[account(
        init,
        payer = authority,
        space = InfoPoolAccount::LEN,
        seeds = [INFO_POOL_SEED, pool_id.as_ref()],
        bump
    )]
    pub info_pool: Account<'info, InfoPoolAccount>,

    #[account(mut)]
    pub authority: Signer<'info>,

    pub system_program: Program<'info, System>,
}

pub fn handler(
    ctx: Context<InitializeInfoPool>,
    pool_id: Pubkey,
) -> Result<()> {
    let info_pool = &mut ctx.accounts.info_pool;

    info_pool.pool_id      = pool_id;
    info_pool.authority    = ctx.accounts.authority.key();
    info_pool.assets       = Vec::new();
    info_pool.pool_size    = 0;
    info_pool.pool_weight  = 1_000_000; // Start at 1.0
    info_pool.last_updated = Clock::get()?.unix_timestamp;
    info_pool.bump         = ctx.bumps.info_pool;

    emit!(InfoPoolInitialized {
        info_pool: ctx.accounts.info_pool.key(),
        pool_id,
    });

    Ok(())
}

// ═══════════════════════════════════════════════════
// REGISTER POOL — Add asset to Info Pool tracking
// Called by Pool Program via CPI when asset is added
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
pub struct RegisterPool<'info> {
    #[account(
        mut,
        seeds = [INFO_POOL_SEED, info_pool.pool_id.as_ref()],
        bump = info_pool.bump,
    )]
    pub info_pool: Account<'info, InfoPoolAccount>,

    /// Pool Program must be the caller
    pub pool_authority: Signer<'info>,
}

pub fn handler_register(
    ctx: Context<RegisterPool>,
) -> Result<()> {
    // Verify the signer is a PDA whose owner is the Pool program.
    // PDAs sign via invoke_signed — their .key() is the PDA address,
    // NOT the program ID. Checking owner (the program that created the account)
    // is the correct authority check for CPI-signed PDAs.
    let expected_program: Pubkey = POOL_PROGRAM_ID
        .parse()
        .map_err(|_| error!(InfoPoolError::NotPoolProgram))?;
    require!(
        *ctx.accounts.pool_authority.to_account_info().owner == expected_program,
        InfoPoolError::NotPoolProgram
    );

    // Pool registration is handled via add_asset_to_info_pool
    // This is a lightweight registration call
    ctx.accounts.info_pool.last_updated =
        Clock::get()?.unix_timestamp;

    Ok(())
}

// ── EVENTS ────────────────────────────────────────
#[event]
pub struct InfoPoolInitialized {
    pub info_pool: Pubkey,
    pub pool_id:   Pubkey,
         }
