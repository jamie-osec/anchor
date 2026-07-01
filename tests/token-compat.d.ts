import BN from "bn.js";
import {
  Connection,
  PublicKey,
  Signer,
  TransactionSignature,
} from "@solana/web3.js";

export class Token {
  public publicKey: PublicKey;

  constructor(
    connection: Connection,
    publicKey: PublicKey,
    programId: PublicKey,
    payer: Signer
  );

  static createMint(
    connection: Connection,
    payer: Signer,
    mintAuthority: PublicKey,
    freezeAuthority: PublicKey | null,
    decimals: number,
    programId?: PublicKey
  ): Promise<Token>;

  static createWrappedNativeAccount(
    connection: Connection,
    programId: PublicKey,
    owner: PublicKey,
    payer: Signer,
    amount: number | string | BN
  ): Promise<PublicKey>;

  static getAssociatedTokenAddress(
    associatedProgramId: PublicKey,
    programId: PublicKey,
    mint: PublicKey,
    owner: PublicKey,
    allowOwnerOffCurve?: boolean
  ): Promise<PublicKey>;

  createAccount(owner: PublicKey): Promise<PublicKey>;
  createAssociatedTokenAccount(owner: PublicKey): Promise<PublicKey>;

  mintTo(
    destination: PublicKey,
    authority: PublicKey,
    multiSigners: Signer[],
    amount: number | string | BN
  ): Promise<TransactionSignature>;

  transfer(
    source: PublicKey,
    destination: PublicKey,
    owner: Signer | PublicKey,
    multiSigners: Signer[],
    amount: number | string | BN
  ): Promise<TransactionSignature>;

  getAccountInfo(address: PublicKey): Promise<{
    amount: BN;
    isInitialized: boolean;
    mint: PublicKey;
    owner: PublicKey;
    [key: string]: unknown;
  }>;

  getMintInfo(): Promise<{
    decimals: number;
    freezeAuthority: PublicKey | null;
    mintAuthority: PublicKey | null;
    supply: BN;
    [key: string]: unknown;
  }>;
}
