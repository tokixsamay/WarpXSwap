use anchor_lang::prelude::*;
use crate::state::*;
use crate::constants::*;
use crate::errors::GovernanceError;

// ═══════════════════════════════════════════════════
// INITIALIZE GOVERNANCE
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
#[instruction(pool_id: Pubkey)]
pub struct InitializeGovernance<'info> {
    #[account(
        init,
        payer = authority,
        space = GovernanceAccount::LEN,
        seeds = [GOVERNANCE_SEED, pool_id.as_ref()],
        bump
    )]
    pub governance: Account<'info, GovernanceAccount>,

    #[account(mut)]
    pub authority: Signer<'info>,

    pub system_program: Program<'info, System>,
}

pub fn handler(
    ctx: Context<InitializeGovernance>,
    pool_id:             Pubkey,
    min_votes_to_pass:   u64,
    execute_delay_secs:  i64,
) -> Result<()> {
    require!(min_votes_to_pass >= 1, GovernanceError::InvalidParameter);
    require!(execute_delay_secs >= 0, GovernanceError::InvalidParameter);

    let gov = &mut ctx.accounts.governance;

    gov.pool_id             = pool_id;
    gov.top_10              = Vec::new();
    gov.total_stake         = 0;
    gov.proposal_count      = 0;
    gov.last_updated        = Clock::get()?.unix_timestamp;
    gov.min_votes_to_pass   = min_votes_to_pass;
    gov.execute_delay_secs  = execute_delay_secs;
    gov.bump                = ctx.bumps.governance;

    emit!(GovernanceInitialized {
        governance: ctx.accounts.governance.key(),
        pool_id,
    });

    Ok(())
}

// ═══════════════════════════════════════════════════
// REGISTER CONTRIBUTOR
// Called by Pool Program via CPI on deposit
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
#[instruction(stake_amount: u64)]
pub struct RegisterContributor<'info> {
    #[account(
        mut,
        seeds = [GOVERNANCE_SEED, governance.pool_id.as_ref()],
        bump = governance.bump,
    )]
    pub governance: Account<'info, GovernanceAccount>,

    #[account(
        init,
        payer = pool_authority,
        space = ContributorAccount::LEN,
        seeds = [
            CONTRIBUTOR_SEED,
            governance.pool_id.as_ref(),
            contributor_wallet.key().as_ref()
        ],
        bump
    )]
    pub contributor_account: Account<'info, ContributorAccount>,

    /// Contributor's wallet (not signer — registered by Pool)
    /// CHECK: Wallet address passed from Pool Program
    pub contributor_wallet: AccountInfo<'info>,

    /// Pool Program must call this — verified via owner check on pool_authority PDA
    #[account(mut)]
    pub pool_authority: Signer<'info>,

    pub system_program: Program<'info, System>,
}

pub fn handler_register(
    ctx: Context<RegisterContributor>,
    stake_amount: u64,
) -> Result<()> {
    // Verify caller is a Pool program PDA (owned by the pool program)
    let pool_id: Pubkey = POOL_PROGRAM_ID
        .parse()
        .map_err(|_| error!(GovernanceError::NotPoolProgram))?;
    require!(
        *ctx.accounts.pool_authority.to_account_info().owner == pool_id,
        GovernanceError::NotPoolProgram
    );

    require!(stake_amount > 0, GovernanceError::ZeroStake);

    let gov         = &mut ctx.accounts.governance;
    let contributor = &mut ctx.accounts.contributor_account;
    let wallet      = ctx.accounts.contributor_wallet.key();

    contributor.contributor   = wallet;
    contributor.pool_id       = gov.pool_id;
    contributor.stake_amount  = stake_amount;
    contributor.last_proposal = 0;
    contributor.voted_on      = Vec::new();
    contributor.is_top_10     = false;
    contributor.bump          = ctx.bumps.contributor_account;

    // Update total stake
    gov.total_stake = gov.total_stake
        .checked_add(stake_amount)
        .ok_or(GovernanceError::MathOverflow)?;

    // Update top 10
    update_top_10(gov, wallet, stake_amount, true)?;

    emit!(ContributorRegistered {
        pool_id:      gov.pool_id,
        contributor:  wallet,
        stake_amount,
    });

    Ok(())
}

// ═══════════════════════════════════════════════════
// UPDATE CONTRIBUTOR STAKE
// Called by Pool Program on deposit/withdraw
// ═══════════════════════════════════════════════════

#[derive(Accounts)]
#[instruction(new_stake: u64)]
pub struct UpdateContributorStake<'info> {
    #[account(
        mut,
        seeds = [GOVERNANCE_SEED, governance.pool_id.as_ref()],
        bump = governance.bump,
    )]
    pub governance: Account<'info, GovernanceAccount>,

    #[account(
        mut,
        seeds = [
            CONTRIBUTOR_SEED,
            governance.pool_id.as_ref(),
            contributor_account.contributor.as_ref()
        ],
        bump = contributor_account.bump,
    )]
    pub contributor_account: Account<'info, ContributorAccount>,

    pub pool_authority: Signer<'info>,
}

pub fn handler_update_stake(
    ctx: Context<UpdateContributorStake>,
    new_stake: u64,
) -> Result<()> {
    // Verify caller is a Pool program PDA (owned by the pool program)
    let pool_id: Pubkey = POOL_PROGRAM_ID
        .parse()
        .map_err(|_| error!(GovernanceError::NotPoolProgram))?;
    require!(
        *ctx.accounts.pool_authority.to_account_info().owner == pool_id,
        GovernanceError::NotPoolProgram
    );

    let gov         = &mut ctx.accounts.governance;
    let contributor = &mut ctx.accounts.contributor_account;

    let old_stake = contributor.stake_amount;
    let wallet    = contributor.contributor;

    // Update total stake
    gov.total_stake = gov.total_stake
        .checked_sub(old_stake)
        .ok_or(GovernanceError::MathOverflow)?
        .checked_add(new_stake)
        .ok_or(GovernanceError::MathOverflow)?;

    contributor.stake_amount = new_stake;

    // Recalculate top 10
    update_top_10(gov, wallet, new_stake, new_stake > 0)?;

    emit!(StakeUpdated {
        pool_id:     gov.pool_id,
        contributor: wallet,
        old_stake,
        new_stake,
    });

    Ok(())
}

// ── HELPER: Update Top 10 ─────────────────────────
pub fn update_top_10(
    gov:      &mut GovernanceAccount,
    wallet:   Pubkey,
    stake:    u64,
    is_add:   bool,
) -> Result<()> {
    if is_add {
        // Add or update in top 10 candidates
        if !gov.top_10.contains(&wallet) && gov.top_10.len() < GovernanceAccount::MAX_TOP_N {
            gov.top_10.push(wallet);
        }
    } else {
        // Remove if stake = 0
        gov.top_10.retain(|&k| k != wallet);
    }

    // V1: top_10 = first MAX_TOP_N contributors with stake > 0.
    // Not sorted by stake amount. V2 should maintain a stake-sorted leaderboard
    // using a separate sorted mapping, so voting power reflects actual stake size.

    Ok(())
}

// ── EVENTS ────────────────────────────────────────
#[event]
pub struct GovernanceInitialized {
    pub governance: Pubkey,
    pub pool_id:    Pubkey,
}

#[event]
pub struct ContributorRegistered {
    pub pool_id:      Pubkey,
    pub contributor:  Pubkey,
    pub stake_amount: u64,
}

#[event]
pub struct StakeUpdated {
    pub pool_id:     Pubkey,
    pub contributor: Pubkey,
    pub old_stake:   u64,
    pub new_stake:   u64,
}
