pub mod instructions;
pub use instructions::*;

use anchor_lang::prelude::*;
use antegen_utils::thread::ThreadResponse;

declare_id!("AgTstktpwF7FfmUpLQW6NLdCioiF4nmQqV9oYjZQzHjj");

#[program]
pub mod antegen_test_program {
    use super::*;

    pub fn close_to(ctx: Context<CloseTo>) -> Result<ThreadResponse> {
        instructions::close_to::handler(ctx)
    }
}
