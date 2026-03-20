# Thread Program Test Coverage

## Summary

| Module | Total | Implemented | Notes |
|--------|-------|-------------|-------|
| state_unit | 28 | 28 | Pure Rust, no SVM |
| config_init | 4 | 4 | |
| config_update | 12 | 12 | |
| thread_create | 19 | 19 | Nonce test excluded (complex LiteSVM setup) |
| fiber_create | 11 | 11 | |
| fiber_update | 5 | 5 | |
| fiber_close | 8 | 8 | |
| thread_update | 10 | 10 | |
| thread_withdraw | 6 | 6 | |
| thread_close | 7 | 7 | |
| thread_delete | 4 | 4 | |
| thread_memo | 8 | 8 | |
| thread_exec | 13 | 13 | CPI-dependent; nonce test excluded |
| **Total** | **135** | **135** | |

## Error Codes Tested

| Error Code | Tests |
|------------|-------|
| InvalidAuthority | config_update, fiber_create, fiber_update, fiber_close, thread_update, thread_close |
| InvalidFeePercentage | config_update (4 tests) |
| InvalidFiberIndex | fiber_create (2 tests) |
| InvalidInstruction | fiber_create, fiber_update |
| InvalidFiberAccount | fiber_update, thread_close |
| FiberAccountRequired | fiber_close |
| MissingFiberAccounts | thread_close |
| InvalidConfigAdmin | thread_delete |
| WithdrawalTooLarge | thread_withdraw |
| ThreadPaused | thread_exec |
| GlobalPauseActive | thread_exec |
| InvalidThreadState | thread_exec (no fibers) |
| TriggerConditionFailed | thread_exec (timestamp not ready) |

## Trigger Types Tested

| Trigger | Create | Update | Exec |
|---------|--------|--------|------|
| Immediate | Y | Y | Y |
| Timestamp | Y | Y | Y |
| Interval | Y | Y | Y |
| Cron | Y | Y | - |
| Slot | Y | Y | Y |
| Epoch | Y | - | - |
| Account | Y | - | - |

## Signal Types Tested

| Signal | thread_memo | thread_exec |
|--------|-------------|-------------|
| None | Y | Y |
| Chain | Y | - |
| Close | Y | Y |
| Repeat | Y | - |
| Next | Y | - |

## LiteSVM Capabilities Used

- `LiteSVM::new()` - Basic SVM creation
- `add_program()` - Loading compiled program
- `airdrop()` - Funding test accounts
- `send_transaction()` - Transaction execution
- `get_account()` - Account data inspection
- `get_sysvar::<Clock>()` / `set_sysvar()` - Clock manipulation
- `warp_to_slot()` - Slot warping
- `latest_blockhash()` - Transaction signing

## Running Tests

```bash
# Build the program first
cd programs/thread && cargo build-sbf

# Run all tests
cargo test --test thread_tests -- --nocapture

# Run specific module
cargo test --test thread_tests config_init -- --nocapture
cargo test --test thread_tests thread_exec -- --nocapture
```
