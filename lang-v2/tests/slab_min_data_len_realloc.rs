use {
    anchor_lang::{
        accounts::{Account, SlabSchema},
        testing::AccountBuffer,
        AccountRealloc, AnchorAccount,
    },
    bytemuck::{Pod, Zeroable},
    pinocchio::account::AccountView,
    solana_program_error::ProgramError,
};

const DATA_OFFSET: usize = 8;
const PHYSICAL_MIN_DATA_LEN: usize = DATA_OFFSET + core::mem::size_of::<CustomHeader>();
const DECLARED_MIN_DATA_LEN: usize = PHYSICAL_MIN_DATA_LEN + 16;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct CustomHeader {
    counter: u64,
    bump: u64,
}

impl SlabSchema for CustomHeader {
    const DATA_OFFSET: usize = DATA_OFFSET;
    const MIN_DATA_LEN: usize = DECLARED_MIN_DATA_LEN;

    fn validate(_view: &AccountView, data: &[u8]) -> Result<(), ProgramError> {
        if data.len() < Self::MIN_DATA_LEN {
            return Err(ProgramError::AccountDataTooSmall);
        }
        Ok(())
    }
}

type CustomAccount = Account<CustomHeader>;

fn setup_account(data_len: usize) -> AccountBuffer<128> {
    let buf = AccountBuffer::<128>::new();
    buf.init([0x44; 32], [0x55; 32], data_len, false, true, false);
    let data = [0u8; 128];
    buf.write_data(&data[..data_len]);
    buf
}

#[test]
fn custom_schema_minimum_exceeds_physical_layout() {
    assert_eq!(PHYSICAL_MIN_DATA_LEN, 24);
    assert_eq!(<CustomAccount as AnchorAccount>::MIN_DATA_LEN, DECLARED_MIN_DATA_LEN);
    assert!(DECLARED_MIN_DATA_LEN > PHYSICAL_MIN_DATA_LEN);
}

#[test]
fn realloc_rejects_shrink_below_custom_schema_minimum() {
    let buf = setup_account(DECLARED_MIN_DATA_LEN);
    let payer = AccountBuffer::<128>::new();
    payer.init([0x99; 32], [0x11; 32], 0, true, true, false);

    let view = unsafe { buf.view() };
    let mut account = unsafe { CustomAccount::load_mut(view) }.unwrap();
    let payer_view = unsafe { payer.view() };

    let err = account
        .realloc_account(PHYSICAL_MIN_DATA_LEN, payer_view, false)
        .expect_err("realloc must reject spaces below the schema minimum");
    assert_eq!(err, ProgramError::AccountDataTooSmall);
    assert_eq!(account.current_space(), DECLARED_MIN_DATA_LEN);
    drop(account);

    CustomAccount::load(unsafe { buf.view() }).expect("account should stay reloadable");
}
