use anchor_lang::prelude::*;
use crate::state::*;
use crate::constants::*;

#[derive(Accounts)]
pub struct InitializeRouter<'info> {
    #[account(
        init,
        payer = authority,
        space = RouterConfig::LEN,
        seeds = [ROUTER_SEED],
        bump
    )]
    pub router_config: Account<'info, RouterConfig>,

    #[account(mut)]
    pub authority: Signer<'info>,

    pub system_program: Program<'info, System>,
}

pub fn handler(ctx: Context<InitializeRouter>) -> Result<()> {
    let config = &mut ctx.accounts.router_config;

    config.info_pool_program = INFO_POOL_PROGRAM_ID
        .parse()
        .map_err(|_| error!(crate::errors::RoutingError::RouterNotActive))?;
    config.pool_program = POOL_PROGRAM_ID
        .parse()
        .map_err(|_| error!(crate::errors::RoutingError::RouterNotActive))?;
    config.is_active         = true;
    config.bump              = ctx.bumps.router_config;

    emit!(RouterInitialized {
        router: ctx.accounts.router_config.key(),
    });

    Ok(())
}

#[event]
pub struct RouterInitialized {
    pub router: Pubkey,
}
