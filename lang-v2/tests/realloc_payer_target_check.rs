use {
    anchor_lang_v2::{cpi::realloc_account, testing::AccountBuffer},
    solana_program_error::ProgramError,
};

const PROGRAM_ID: [u8; 32] = [0x42; 32];

#[test]
fn realloc_rejects_payer_equal_target_before_resize() {
    let buf = AccountBuffer::<256>::new();
    buf.init([0xAB; 32], PROGRAM_ID, 24, false, true, false);

    let mut account = unsafe { buf.view() };
    let payer = account;
    let old_space = account.data_len();
    let old_lamports = account.lamports();

    let err = realloc_account(&mut account, 64, &payer, false)
        .expect_err("realloc must reject using the target account as the payer");

    assert_eq!(err, ProgramError::InvalidArgument);
    assert_eq!(account.data_len(), old_space);
    assert_eq!(account.lamports(), old_lamports);
}
