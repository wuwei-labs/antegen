use anchor_lang::prelude::*;
use antegen_thread_program::fiber;

declare_id!("FncKotqKRzg4D8T6pBj1cPz1mJgQb1YGzkPFh8SEAEXo");

#[program]
pub mod reentrance_test {
    use super::*;

    /// Called during thread_exec via invoke_signed.
    /// CPIs to fiber_program::update_fiber to prove reentrancy works.
    /// Thread PDA signer status is propagated from the outer invoke_signed.
    pub fn cpi_update_fiber(ctx: Context<CpiUpdateFiber>, fiber_index: u8) -> Result<()> {
        // Build a simple instruction to update the fiber with
        let simple_ix = antegen_thread_program::state::SerializableInstruction {
            program_id: ctx.accounts.thread.key(),
            accounts: vec![],
            data: vec![0xAB, 0xCD], // arbitrary marker data
        };

        fiber::cpi::update_fiber(
            CpiContext::new(
                ctx.accounts.fiber_program.key(),
                fiber::cpi::accounts::FiberUpdate {
                    thread: ctx.accounts.thread.to_account_info(),
                    fiber: ctx.accounts.fiber.to_account_info(),
                    system_program: ctx.accounts.system_program.to_account_info(),
                },
            ),
            fiber_index,
            simple_ix,
            Some(42),
        )?;

        // Return Signal::None so thread_exec continues normally
        // Signal::None = borsh enum variant 0 = [0]
        anchor_lang::solana_program::program::set_return_data(&[0]);

        Ok(())
    }

    /// Called during thread_exec via invoke_signed.
    /// CPIs to fiber_program::close_fiber to prove reentrancy works.
    pub fn cpi_close_fiber(ctx: Context<CpiCloseFiber>) -> Result<()> {
        fiber::cpi::close_fiber(CpiContext::new(
            ctx.accounts.fiber_program.key(),
            fiber::cpi::accounts::FiberClose {
                thread: ctx.accounts.thread.to_account_info(),
                fiber: ctx.accounts.fiber.to_account_info(),
            },
        ))?;

        // Return Signal::None
        anchor_lang::solana_program::program::set_return_data(&[0]);

        Ok(())
    }

    /// Called during thread_exec via invoke_signed.
    /// CPIs to fiber_program::swap_fiber to prove reentrancy works.
    pub fn cpi_swap_fiber(ctx: Context<CpiSwapFiber>) -> Result<()> {
        fiber::cpi::swap_fiber(CpiContext::new(
            ctx.accounts.fiber_program.key(),
            fiber::cpi::accounts::FiberSwap {
                thread: ctx.accounts.thread.to_account_info(),
                target: ctx.accounts.target.to_account_info(),
                source: ctx.accounts.source.to_account_info(),
            },
        ))?;

        // Return Signal::None
        anchor_lang::solana_program::program::set_return_data(&[0]);

        Ok(())
    }
}

#[derive(Accounts)]
pub struct CpiUpdateFiber<'info> {
    /// Thread PDA — signer (propagated from invoke_signed in thread_exec)
    #[account(mut)]
    pub thread: Signer<'info>,

    /// CHECK: Fiber to update — passed through to Fiber Program
    #[account(mut)]
    pub fiber: UncheckedAccount<'info>,

    /// CHECK: Fiber Program
    pub fiber_program: UncheckedAccount<'info>,

    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct CpiCloseFiber<'info> {
    /// Thread PDA — signer, receives rent back
    #[account(mut)]
    pub thread: Signer<'info>,

    /// CHECK: Fiber to close — passed through to Fiber Program
    #[account(mut)]
    pub fiber: UncheckedAccount<'info>,

    /// CHECK: Fiber Program
    pub fiber_program: UncheckedAccount<'info>,
}

#[derive(Accounts)]
pub struct CpiSwapFiber<'info> {
    /// Thread PDA — signer, receives source rent back
    #[account(mut)]
    pub thread: Signer<'info>,

    /// CHECK: Target fiber — passed through to Fiber Program
    #[account(mut)]
    pub target: UncheckedAccount<'info>,

    /// CHECK: Source fiber — passed through to Fiber Program
    #[account(mut)]
    pub source: UncheckedAccount<'info>,

    /// CHECK: Fiber Program
    pub fiber_program: UncheckedAccount<'info>,
}
