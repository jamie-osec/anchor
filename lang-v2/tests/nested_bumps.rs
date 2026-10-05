#![allow(dead_code)]

use anchor_lang::prelude::*;

declare_id!("11111111111111111111111111111111");

#[derive(Accounts)]
pub struct Inner {
    #[account(seeds = [b"vault"], bump)]
    pub vault: UncheckedAccount,
}

#[derive(Accounts)]
pub struct Outer {
    pub authority: UncheckedAccount,
    pub inner: Nested<Inner>,
}

fn nested_bump(ctx: &Context<'_, Outer>) -> u8 {
    ctx.bumps.inner.vault
}

#[test]
fn nested_bumps_are_available_through_context() {
    let _: fn(&Context<'_, Outer>) -> u8 = nested_bump;
}

mod shared {
    use super::*;
    #[derive(Accounts)]
    pub struct Common {
        #[account(mut)]
        pub writable: UncheckedAccount,
        pub signer: Signer,
    }
}

type CommonAlias = shared::Common;
type OptionalSigner = Option<Signer>;

#[derive(Accounts)]
pub struct Automatic {
    pub before: UncheckedAccount,
    pub common: CommonAlias,
    pub absent: OptionalSigner,
    pub after: UncheckedAccount,
}

#[test]
fn automatic_nesting_preserves_runtime_client_and_cpi_order() {
    use anchor_lang::testing::{AccountBuffer, MIN_ACCOUNT_BUF};
    let buffers: [AccountBuffer<MIN_ACCOUNT_BUF>; 5] = core::array::from_fn(|i| {
        let buffer = AccountBuffer::new();
        buffer.init(
            if i == 3 {
                ID.to_bytes()
            } else {
                [i as u8 + 1; 32]
            },
            [0; 32],
            0,
            i == 2,
            i == 1,
            false,
        );
        buffer
    });
    let views = buffers.each_ref().map(|buffer| unsafe { buffer.view() });
    let (accounts, bumps, ()) = Automatic::try_accounts(&ID, &views, None, 0, &[]).unwrap();
    assert_eq!(<Automatic as TryAccounts>::HEADER_SIZE, 5);
    assert_eq!(<Automatic as TryAccounts>::MUT_MASK, [2, 0, 0, 0]);
    assert_eq!(accounts.common.signer.address(), views[2].address());
    assert!(accounts.absent.is_none());
    assert_eq!(accounts.after.address(), views[4].address());
    let _: () = bumps.common.signer;

    let client = __client_accounts_automatic::Automatic {
        before: *views[0].address(),
        common: shared::__client_accounts_common::Common {
            writable: *views[1].address(),
            signer: *views[2].address(),
        },
        absent: None,
        after: *views[4].address(),
    };
    let metas = client.to_account_metas(None);
    assert_eq!(
        metas.iter().map(|m| m.pubkey).collect::<Vec<_>>(),
        views.iter().map(|v| *v.address()).collect::<Vec<_>>()
    );
    assert!(metas[1].is_writable);
    assert!(metas[2].is_signer);
    assert!(!metas[3].is_signer);
    assert!(!client.to_account_metas(Some(false))[2].is_signer);

    let mut writable = views[1];
    let cpi = __cpi_accounts_automatic::Automatic {
        before: views[0].to_cpi_handle(),
        common: shared::__cpi_accounts_common::Common {
            writable: writable.to_cpi_handle_mut(),
            signer: views[2].to_cpi_handle(),
        },
        absent: None,
        after: views[4].to_cpi_handle(),
    };
    let cpi_metas = cpi.to_instruction_accounts();
    assert_eq!(cpi_metas.len(), 5);
    assert!(cpi_metas[1].is_writable);
    assert!(cpi_metas[2].is_signer);
    assert_eq!(cpi.to_cpi_handles().len(), 4);
    assert_eq!(
        cpi.optional_account_sentinel_flags(),
        [false, false, false, true, false]
    );
}

#[cfg(feature = "idl-build")]
#[test]
fn automatic_nesting_composes_idl_metadata() {
    let json = Automatic::__idl_accounts();
    assert!(json.contains("\"name\":\"writable\",\"writable\":true"));
    assert!(json.contains("\"name\":\"signer\",\"signer\":true"));
    assert!(json.contains("\"name\":\"absent\",\"signer\":true,\"optional\":true"));
}

type SystemProgramAlias = Program<System>;
#[derive(Accounts)]
pub struct AliasedResolved {
    #[account(resolve)]
    pub system_program: SystemProgramAlias,
    pub optional: OptionalSigner,
}

#[test]
fn resolved_program_alias_uses_trait_address_and_preserves_optional_presence() {
    let client = __client_accounts_aliasedresolved::AliasedResolvedResolved { optional: None };
    let metas = client.to_account_metas(None);
    assert_eq!(metas[0].pubkey, System::id());
    assert_eq!(metas[1].pubkey, ID);
    assert!(!metas[1].is_signer);
}
