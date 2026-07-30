#![cfg(feature = "metadata")]

use {
    anchor_lang_v2::{testing::AccountBuffer, AccountDeserialize, AnchorAccount},
    anchor_spl_v2::metadata::{self, MetadataAccount, TokenRecordAccount},
    borsh::to_vec,
    solana_program_error::ProgramError,
    solana_pubkey::Pubkey,
};

fn sample_metadata() -> mpl_token_metadata::accounts::Metadata {
    let creator = Pubkey::from([7u8; 32]);
    mpl_token_metadata::accounts::Metadata {
        key: mpl_token_metadata::types::Key::MetadataV1,
        update_authority: Pubkey::from([1u8; 32]),
        mint: Pubkey::from([2u8; 32]),
        name: "Pump AMM".to_string(),
        symbol: "PUMP".to_string(),
        uri: "https://example.invalid/pump.json".to_string(),
        seller_fee_basis_points: 250,
        creators: Some(vec![mpl_token_metadata::types::Creator {
            address: creator,
            verified: true,
            share: 100,
        }]),
        primary_sale_happened: false,
        is_mutable: true,
        edition_nonce: Some(255),
        token_standard: None,
        collection: None,
        uses: None,
        collection_details: None,
        programmable_config: None,
    }
}

fn sample_token_record() -> mpl_token_metadata::accounts::TokenRecord {
    mpl_token_metadata::accounts::TokenRecord {
        key: mpl_token_metadata::types::Key::TokenRecord,
        bump: 7,
        state: mpl_token_metadata::types::TokenState::Unlocked,
        rule_set_revision: Some(9),
        delegate: Some(Pubkey::from([3u8; 32])),
        delegate_role: Some(mpl_token_metadata::types::TokenDelegateRole::Transfer),
        locked_transfer: Some(Pubkey::from([4u8; 32])),
    }
}

fn sample_legacy_token_record_bytes() -> Vec<u8> {
    let record = sample_token_record();
    let mut data = Vec::new();
    data.extend_from_slice(&to_vec(&record.key).unwrap());
    data.extend_from_slice(&to_vec(&record.bump).unwrap());
    data.extend_from_slice(&to_vec(&record.state).unwrap());
    data.extend_from_slice(&to_vec(&record.rule_set_revision).unwrap());
    data.extend_from_slice(&to_vec(&record.delegate).unwrap());
    data.extend_from_slice(&to_vec(&record.delegate_role).unwrap());
    data
}

#[test]
fn fixture_is_real_metadata_program_elf() {
    let fixture = include_bytes!("fixtures/metaplex_token_metadata.so");
    assert_eq!(fixture.len(), 283_512);
    assert_eq!(&fixture[..4], b"\x7fELF");
}

#[test]
fn metadata_account_deserializes_raw_metaplex_bytes() {
    let expected = sample_metadata();
    let data = to_vec(&expected).unwrap();
    let account = MetadataAccount::try_deserialize(&mut data.as_slice()).unwrap();

    assert_eq!(account.key, mpl_token_metadata::types::Key::MetadataV1);
    assert_eq!(account.name, "Pump AMM");
    assert_eq!(account.mint, expected.mint);
    assert_eq!(
        account.creators.as_ref().unwrap()[0].address,
        Pubkey::from([7u8; 32])
    );
}

#[test]
fn metadata_account_load_validates_owner_and_raw_data() {
    let expected = sample_metadata();
    let data = to_vec(&expected).unwrap();
    let account = AccountBuffer::<4096>::new();
    account.init(
        [9u8; 32],
        metadata::ID.to_bytes(),
        data.len(),
        false,
        false,
        false,
    );
    account.write_data(&data);

    let loaded = MetadataAccount::load(unsafe { account.view() }).unwrap();
    assert_eq!(loaded.update_authority, expected.update_authority);
    assert_eq!(loaded.seller_fee_basis_points, 250);
}

#[test]
fn metadata_account_rejects_wrong_owner() {
    let data = to_vec(&sample_metadata()).unwrap();
    let account = AccountBuffer::<4096>::new();
    account.init([9u8; 32], [3u8; 32], data.len(), false, false, false);
    account.write_data(&data);

    let err = MetadataAccount::load(unsafe { account.view() }).unwrap_err();
    assert_eq!(err, ProgramError::IllegalOwner);
}

#[test]
fn metadata_account_rejects_non_metadata_key_without_anchor_discriminator() {
    let mut data = to_vec(&sample_metadata()).unwrap();
    data[0] = mpl_token_metadata::types::Key::MasterEditionV2 as u8;

    let err = MetadataAccount::try_deserialize(&mut data.as_slice()).unwrap_err();
    assert_eq!(err, ProgramError::InvalidAccountData);
}

#[test]
fn token_record_account_deserialize_consumes_full_prefix_with_trailing_bytes() {
    let mut data = to_vec(&sample_token_record()).unwrap();
    let trailing = [0xAA, 0xBB, 0xCC];
    data.extend_from_slice(&trailing);

    let mut cursor = data.as_slice();
    let account = TokenRecordAccount::try_deserialize(&mut cursor).unwrap();

    assert_eq!(account.key, mpl_token_metadata::types::Key::TokenRecord);
    assert_eq!(account.bump, 7);
    assert_eq!(account.locked_transfer, Some(Pubkey::from([4u8; 32])));
    assert_eq!(cursor, trailing.as_slice());
}

#[test]
fn token_record_account_deserialize_consumes_legacy_prefix_with_trailing_bytes() {
    let mut data = sample_legacy_token_record_bytes();
    let trailing = [0xFF, 0xEE, 0xDD];
    data.extend_from_slice(&trailing);

    let mut cursor = data.as_slice();
    let account = TokenRecordAccount::try_deserialize(&mut cursor).unwrap();

    assert_eq!(account.key, mpl_token_metadata::types::Key::TokenRecord);
    assert_eq!(account.bump, 7);
    assert_eq!(account.locked_transfer, None);
    assert_eq!(cursor, trailing.as_slice());
}
