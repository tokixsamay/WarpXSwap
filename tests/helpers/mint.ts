/**
 * Bankrun-compatible SPL token helpers.
 *
 * These wrap @solana/spl-token instructions into Bankrun transactions
 * so tests can create mints and fund accounts without a live validator.
 */

import {
  PublicKey,
  Keypair,
  Transaction,
  SystemProgram,
  LAMPORTS_PER_SOL,
} from "@solana/web3.js";
import {
  TOKEN_PROGRAM_ID,
  MintLayout,
  AccountLayout,
  createInitializeMintInstruction,
  createInitializeAccountInstruction,
  createMintToInstruction,
  getAssociatedTokenAddressSync,
  createAssociatedTokenAccountInstruction,
  ASSOCIATED_TOKEN_PROGRAM_ID,
} from "@solana/spl-token";
import { BankrunProvider } from "anchor-bankrun";
import type { ProgramTestContext } from "solana-bankrun";

export const MINT_SIZE      = MintLayout.span;
export const TOKEN_ACC_SIZE = AccountLayout.span;

// ── Mint creation ──────────────────────────────────────────────

export async function createMintBankrun(
  ctx:       ProgramTestContext,
  authority: Keypair,
  decimals:  number,
): Promise<PublicKey> {
  const mintKp     = Keypair.generate();
  const client     = ctx.banksClient;
  const rentExempt = Number(await client.getRent());

  // Estimate rent for mint account
  const mintRent   = Math.ceil(rentExempt * MINT_SIZE / 1000); // simplified

  const tx = new Transaction();
  tx.add(
    SystemProgram.createAccount({
      fromPubkey:           authority.publicKey,
      newAccountPubkey:     mintKp.publicKey,
      space:                MINT_SIZE,
      lamports:             Math.max(mintRent, 1_500_000), // at least 0.0015 SOL
      programId:            TOKEN_PROGRAM_ID,
    }),
    createInitializeMintInstruction(
      mintKp.publicKey,
      decimals,
      authority.publicKey,
      null,                // no freeze authority
    ),
  );

  const [blockhash] = await client.getLatestBlockhash();
  tx.recentBlockhash = blockhash;
  tx.feePayer        = authority.publicKey;
  tx.sign(authority, mintKp);
  await client.processTransaction(tx);

  return mintKp.publicKey;
}

// ── Token account creation ─────────────────────────────────────

export async function createTokenAccountBankrun(
  ctx:       ProgramTestContext,
  mint:      PublicKey,
  owner:     PublicKey,
  payer:     Keypair,
): Promise<PublicKey> {
  const ata    = getAssociatedTokenAddressSync(mint, owner, true);
  const client = ctx.banksClient;

  const tx = new Transaction();
  tx.add(
    createAssociatedTokenAccountInstruction(
      payer.publicKey,
      ata,
      owner,
      mint,
    ),
  );

  const [blockhash] = await client.getLatestBlockhash();
  tx.recentBlockhash = blockhash;
  tx.feePayer        = payer.publicKey;
  tx.sign(payer);
  await client.processTransaction(tx);

  return ata;
}

// ── Token minting ──────────────────────────────────────────────

export async function mintTokensBankrun(
  ctx:       ProgramTestContext,
  mint:      PublicKey,
  dest:      PublicKey,
  amount:    bigint,
  authority: Keypair,
): Promise<void> {
  const client = ctx.banksClient;

  const tx = new Transaction();
  tx.add(
    createMintToInstruction(
      mint,
      dest,
      authority.publicKey,
      amount,
    ),
  );

  const [blockhash] = await client.getLatestBlockhash();
  tx.recentBlockhash = blockhash;
  tx.feePayer        = authority.publicKey;
  tx.sign(authority);
  await client.processTransaction(tx);
}

// ── Account balance reader ─────────────────────────────────────

export async function getTokenBalanceBankrun(
  ctx:     ProgramTestContext,
  account: PublicKey,
): Promise<bigint> {
  const info = await ctx.banksClient.getAccount(account);
  if (!info || info.data.length < AccountLayout.span) return 0n;
  const decoded = AccountLayout.decode(Buffer.from(info.data));
  return decoded.amount;
}

// ── Pre-fund a batch of accounts ──────────────────────────────

export async function fundAccountsBankrun(
  ctx:      ProgramTestContext,
  accounts: Array<{ pubkey: PublicKey; lamports: number }>,
  payer:    Keypair,
): Promise<void> {
  const client = ctx.banksClient;
  const tx     = new Transaction();

  for (const { pubkey, lamports } of accounts) {
    tx.add(
      SystemProgram.transfer({
        fromPubkey: payer.publicKey,
        toPubkey:   pubkey,
        lamports,
      }),
    );
  }

  const [blockhash] = await client.getLatestBlockhash();
  tx.recentBlockhash = blockhash;
  tx.feePayer        = payer.publicKey;
  tx.sign(payer);
  await client.processTransaction(tx);
}

// ── ATA helper ─────────────────────────────────────────────────

export function ata(mint: PublicKey, owner: PublicKey, allowOwnerOffCurve = false): PublicKey {
  return getAssociatedTokenAddressSync(mint, owner, allowOwnerOffCurve);
}
  
