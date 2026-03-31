//! Ported from https://github.com/blueshift-gg/quasar/tree/174901d/examples/multisig/

use anchor_lang::{
    prelude::*,
    system_program::{transfer, Transfer},
};

declare_id!("5KkVfSkB1P64QL4Qis2SvMZNyyM2aUGBM3CynZnACnFg");

const MAX_LABEL_LEN: usize = 32;
const MAX_SIGNERS: usize = 10;

#[program]
pub mod multisig {
    use super::*;

    pub fn create(ctx: Context<Create>, threshold: u8) -> Result<()> {
        let mut signers = Vec::with_capacity(ctx.remaining_accounts.len());

        for account in ctx.remaining_accounts {
            require!(signers.len() < MAX_SIGNERS, ErrorCode::TooManySigners);
            require!(account.is_signer, ErrorCode::MissingRequiredSignature);
            signers.push(*account.key);
        }

        require!(threshold > 0, ErrorCode::InvalidThreshold);
        require!(
            usize::from(threshold) <= signers.len(),
            ErrorCode::InvalidThreshold
        );

        let config = &mut ctx.accounts.config;
        config.creator = ctx.accounts.creator.key();
        config.threshold = threshold;
        config.bump = ctx.bumps.config;
        config.label = String::new();
        config.signers = signers;

        Ok(())
    }

    pub fn deposit(ctx: Context<Deposit>, amount: u64) -> Result<()> {
        let cpi_accounts = Transfer {
            from: ctx.accounts.depositor.to_account_info(),
            to: ctx.accounts.vault.to_account_info(),
        };
        let cpi_ctx = CpiContext::new(ctx.accounts.system_program.key(), cpi_accounts);

        transfer(cpi_ctx, amount)
    }

    pub fn set_label(ctx: Context<SetLabel>, label: String) -> Result<()> {
        require!(label.len() <= MAX_LABEL_LEN, ErrorCode::LabelTooLong);
        ctx.accounts.config.label = label;
        Ok(())
    }

    pub fn execute_transfer(ctx: Context<ExecuteTransfer>, amount: u64) -> Result<()> {
        let mut approvals = 0u8;

        for account in ctx.remaining_accounts {
            if !account.is_signer {
                continue;
            }

            if ctx
                .accounts
                .config
                .signers
                .iter()
                .any(|signer| signer == account.key)
            {
                approvals = approvals.saturating_add(1);
            }
        }

        require!(
            approvals >= ctx.accounts.config.threshold,
            ErrorCode::MissingRequiredSignature
        );

        let config_key = ctx.accounts.config.key();
        let vault_bump = ctx.bumps.vault;
        let signer_seeds: &[&[u8]] = &[b"vault", config_key.as_ref(), &[vault_bump]];
        let signer = &[signer_seeds];
        let cpi_accounts = Transfer {
            from: ctx.accounts.vault.to_account_info(),
            to: ctx.accounts.recipient.to_account_info(),
        };
        let cpi_ctx =
            CpiContext::new_with_signer(ctx.accounts.system_program.key(), cpi_accounts, signer);

        transfer(cpi_ctx, amount)
    }
}

#[derive(Accounts)]
pub struct Create<'info> {
    #[account(mut)]
    pub creator: Signer<'info>,
    #[account(
        init,
        payer = creator,
        space = 8 + MultisigConfig::INIT_SPACE,
        seeds = [b"multisig", creator.key().as_ref()],
        bump
    )]
    pub config: Account<'info, MultisigConfig>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct Deposit<'info> {
    #[account(mut)]
    pub depositor: Signer<'info>,
    pub config: Account<'info, MultisigConfig>,
    #[account(mut, seeds = [b"vault", config.key().as_ref()], bump)]
    pub vault: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
}

#[derive(Accounts)]
pub struct SetLabel<'info> {
    #[account(mut)]
    pub creator: Signer<'info>,
    #[account(
        mut,
        has_one = creator,
        seeds = [b"multisig", creator.key().as_ref()],
        bump = config.bump
    )]
    pub config: Account<'info, MultisigConfig>,
}

#[derive(Accounts)]
pub struct ExecuteTransfer<'info> {
    #[account(
        seeds = [b"multisig", creator.key().as_ref()],
        bump = config.bump,
        has_one = creator
    )]
    pub config: Account<'info, MultisigConfig>,
    pub creator: UncheckedAccount<'info>,
    #[account(mut, seeds = [b"vault", config.key().as_ref()], bump)]
    pub vault: UncheckedAccount<'info>,
    #[account(mut)]
    pub recipient: UncheckedAccount<'info>,
    pub system_program: Program<'info, System>,
}

#[account]
#[derive(InitSpace)]
pub struct MultisigConfig {
    pub creator: Pubkey,
    pub threshold: u8,
    pub bump: u8,
    #[max_len(MAX_LABEL_LEN)]
    pub label: String,
    #[max_len(MAX_SIGNERS)]
    pub signers: Vec<Pubkey>,
}

#[error_code]
pub enum ErrorCode {
    #[msg("At least one signer is required and the threshold cannot exceed signer count.")]
    InvalidThreshold,
    #[msg("Too many signers were provided.")]
    TooManySigners,
    #[msg("A required signer was missing.")]
    MissingRequiredSignature,
    #[msg("The label exceeds the maximum supported length.")]
    LabelTooLong,
}
