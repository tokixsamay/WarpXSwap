use anchor_lang::prelude::*;

pub mod instructions;
pub mod state;
pub mod errors;
pub mod constants;

use instructions::*;

declare_id!("3fdt9Skkj52bMvutU56CuBMZhrUsaStXBxGNtDPVCRSG");

#[program]
pub mod routing_program {
    use super::*;

    // ── SETUP ─────────────────────────────────────

    pub fn initialize_router(
        ctx: Context<InitializeRouter>,
    ) -> Result<()> {
        instructions::initialize::handler(ctx)
    }

    // ── CORE ROUTING ──────────────────────────────

    pub fn find_best_pool(
        ctx: Context<FindBestPool>,
        params: FindBestPoolParams,
    ) -> Result<RouteResult> {
        instructions::routing::handler_find_best(ctx, params)
    }

    pub fn get_quote(
        ctx: Context<GetQuote>,
        params: QuoteParams,
    ) -> Result<QuoteResult> {
        instructions::routing::handler_get_quote(ctx, params)
    }

    // ── EXECUTE ROUTE ─────────────────────────────
    // Find best pool + execute swap in one transaction

    pub fn execute_route(
        ctx: Context<ExecuteRoute>,
        params: ExecuteRouteParams,
    ) -> Result<RouteResult> {
        instructions::execute::handler(ctx, params)
    }


          }
