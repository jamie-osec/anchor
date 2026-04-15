import * as anchor from "@coral-xyz/anchor";
import { Program } from "@coral-xyz/anchor";
import {
  PublicKey,
  Keypair,
  SystemProgram,
  sendAndConfirmTransaction,
  Transaction,
} from "@solana/web3.js";
import { TokenExtensions } from "../target/types/token_extensions";
import { ASSOCIATED_PROGRAM_ID } from "@coral-xyz/anchor/dist/cjs/utils/token";
import {
  ExtensionType,
  createInitializeMintInstruction,
  createInitializeAccountInstruction,
  getAccountLen,
  getMintLen,
} from "@solana/spl-token";
import { it } from "node:test";

const TOKEN_2022_PROGRAM_ID = new anchor.web3.PublicKey(
  "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb"
);

export function associatedAddress({
  mint,
  owner,
}: {
  mint: PublicKey;
  owner: PublicKey;
}): PublicKey {
  return PublicKey.findProgramAddressSync(
    [owner.toBuffer(), TOKEN_2022_PROGRAM_ID.toBuffer(), mint.toBuffer()],
    ASSOCIATED_PROGRAM_ID
  )[0];
}

describe("token extensions", () => {
  const provider = anchor.AnchorProvider.env();
  anchor.setProvider(provider);

  const program = anchor.workspace.TokenExtensions as Program<TokenExtensions>;

  const payer = Keypair.generate();

  it("airdrop payer", async () => {
    await provider.connection.confirmTransaction(
      await provider.connection.requestAirdrop(payer.publicKey, 10000000000),
      "confirmed"
    );
  });

  let mint = new Keypair();

  it("Create mint account test passes", async () => {
    const [extraMetasAccount] = PublicKey.findProgramAddressSync(
      [
        anchor.utils.bytes.utf8.encode("extra-account-metas"),
        mint.publicKey.toBuffer(),
      ],
      program.programId
    );
    await program.methods
      .createMintAccount({
        name: "hello",
        symbol: "hi",
        uri: "https://hi.com",
      })
      .accountsStrict({
        payer: payer.publicKey,
        authority: payer.publicKey,
        receiver: payer.publicKey,
        mint: mint.publicKey,
        mintTokenAccount: associatedAddress({
          mint: mint.publicKey,
          owner: payer.publicKey,
        }),
        extraMetasAccount: extraMetasAccount,
        systemProgram: anchor.web3.SystemProgram.programId,
        associatedTokenProgram: ASSOCIATED_PROGRAM_ID,
        tokenProgram: TOKEN_2022_PROGRAM_ID,
      })
      .signers([mint, payer])
      .rpc();
  });

  it("mint extension constraints test passes", async () => {
    await program.methods
      .checkMintExtensionsConstraints()
      .accountsStrict({
        authority: payer.publicKey,
        mint: mint.publicKey,
      })
      .signers([payer])
      .rpc();
  });

  describe("cpi guard", () => {
    const cpiGuardMint = Keypair.generate();
    const tokenAccount = Keypair.generate();

    it("Create mint and token account with CPI guard extension", async () => {
      const mintLen = getMintLen([]);
      const mintLamports =
        await provider.connection.getMinimumBalanceForRentExemption(mintLen);

      const accountLen = getAccountLen([ExtensionType.CpiGuard]);
      const accountLamports =
        await provider.connection.getMinimumBalanceForRentExemption(accountLen);

      const transaction = new Transaction().add(
        SystemProgram.createAccount({
          fromPubkey: payer.publicKey,
          newAccountPubkey: cpiGuardMint.publicKey,
          space: mintLen,
          lamports: mintLamports,
          programId: TOKEN_2022_PROGRAM_ID,
        }),
        createInitializeMintInstruction(
          cpiGuardMint.publicKey,
          0,
          payer.publicKey,
          null,
          TOKEN_2022_PROGRAM_ID
        ),
        SystemProgram.createAccount({
          fromPubkey: payer.publicKey,
          newAccountPubkey: tokenAccount.publicKey,
          space: accountLen,
          lamports: accountLamports,
          programId: TOKEN_2022_PROGRAM_ID,
        }),
        createInitializeAccountInstruction(
          tokenAccount.publicKey,
          cpiGuardMint.publicKey,
          payer.publicKey,
          TOKEN_2022_PROGRAM_ID
        )
      );

      await sendAndConfirmTransaction(provider.connection, transaction, [
        payer,
        cpiGuardMint,
        tokenAccount,
      ]);
    });

    it("Enable CPI guard via program CPI", async () => {
      await program.methods
        .enableCpiGuard()
        .accountsStrict({
          tokenProgram: TOKEN_2022_PROGRAM_ID,
          tokenAccount: tokenAccount.publicKey,
          owner: payer.publicKey,
        })
        .signers([payer])
        .rpc();
    });

    it("Disable CPI guard via program CPI", async () => {
      await program.methods
        .disableCpiGuard()
        .accountsStrict({
          tokenProgram: TOKEN_2022_PROGRAM_ID,
          tokenAccount: tokenAccount.publicKey,
          owner: payer.publicKey,
        })
        .signers([payer])
        .rpc();
    });
  });
});
