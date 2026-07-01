const BN = require("bn.js");
const {
  Keypair,
  SystemProgram,
  Transaction,
  sendAndConfirmTransaction,
} = require("@solana/web3.js");
const {
  ACCOUNT_SIZE,
  NATIVE_MINT,
  TOKEN_PROGRAM_ID,
  createAccount,
  createAssociatedTokenAccount,
  createInitializeAccountInstruction,
  createMint,
  createSyncNativeInstruction,
  getAccount,
  getAssociatedTokenAddress,
  getMint,
  mintTo,
  transfer,
} = require("@solana/spl-token");

const toAmount = (amount) => BigInt(amount.toString());

const withBnAmount = (account) => ({
  ...account,
  amount: new BN(account.amount.toString()),
});

const withBnSupply = (mint) => ({
  ...mint,
  supply: new BN(mint.supply.toString()),
});

class Token {
  constructor(connection, publicKey, programId = TOKEN_PROGRAM_ID, payer) {
    this.connection = connection;
    this.publicKey = publicKey;
    this.programId = programId;
    this.payer = payer;
  }

  static async createMint(
    connection,
    payer,
    mintAuthority,
    freezeAuthority,
    decimals,
    programId = TOKEN_PROGRAM_ID
  ) {
    const publicKey = await createMint(
      connection,
      payer,
      mintAuthority,
      freezeAuthority,
      decimals,
      undefined,
      undefined,
      programId
    );
    return new Token(connection, publicKey, programId, payer);
  }

  static async createWrappedNativeAccount(
    connection,
    programId,
    owner,
    payer,
    amount
  ) {
    const account = Keypair.generate();
    const lamports =
      Number(toAmount(amount)) +
      (await connection.getMinimumBalanceForRentExemption(ACCOUNT_SIZE));

    const tx = new Transaction().add(
      SystemProgram.createAccount({
        fromPubkey: payer.publicKey,
        newAccountPubkey: account.publicKey,
        lamports,
        space: ACCOUNT_SIZE,
        programId,
      }),
      createInitializeAccountInstruction(
        account.publicKey,
        NATIVE_MINT,
        owner,
        programId
      ),
      createSyncNativeInstruction(account.publicKey, programId)
    );
    await sendAndConfirmTransaction(connection, tx, [payer, account]);
    return account.publicKey;
  }

  static async getAssociatedTokenAddress(
    associatedProgramId,
    programId,
    mint,
    owner,
    allowOwnerOffCurve = false
  ) {
    return getAssociatedTokenAddress(
      mint,
      owner,
      allowOwnerOffCurve,
      programId,
      associatedProgramId
    );
  }

  async createAccount(owner) {
    return createAccount(
      this.connection,
      this.payer,
      this.publicKey,
      owner,
      Keypair.generate(),
      undefined,
      this.programId
    );
  }

  async createAssociatedTokenAccount(owner) {
    return createAssociatedTokenAccount(
      this.connection,
      this.payer,
      this.publicKey,
      owner,
      undefined,
      this.programId
    );
  }

  async mintTo(destination, authority, multiSigners, amount) {
    return mintTo(
      this.connection,
      this.payer,
      this.publicKey,
      destination,
      authority,
      toAmount(amount),
      multiSigners,
      undefined,
      this.programId
    );
  }

  async transfer(source, destination, owner, multiSigners, amount) {
    return transfer(
      this.connection,
      this.payer,
      source,
      destination,
      owner,
      toAmount(amount),
      multiSigners,
      undefined,
      this.programId
    );
  }

  async getAccountInfo(address) {
    const account = await getAccount(
      this.connection,
      address,
      undefined,
      this.programId
    );
    return withBnAmount(account);
  }

  async getMintInfo() {
    const mint = await getMint(
      this.connection,
      this.publicKey,
      undefined,
      this.programId
    );
    return withBnSupply(mint);
  }
}

module.exports = { Token };
