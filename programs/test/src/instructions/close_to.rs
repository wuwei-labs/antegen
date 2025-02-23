use anchor_lang::prelude::*;
use antegen_utils::thread::ThreadResponse;

#[derive(Accounts)]
pub struct CloseTo<'info> {
    /// CHECK:
    #[account(mut)]
    pub to: AccountInfo<'info>
}

pub fn handler(
  ctx: Context<CloseTo>
) -> Result<ThreadResponse> {
  let to: &mut AccountInfo = &mut ctx.accounts.to;

  msg!("close to test");
  Ok(ThreadResponse {
    close_to: Some(to.key()),
    ..ThreadResponse::default()
  })
}
