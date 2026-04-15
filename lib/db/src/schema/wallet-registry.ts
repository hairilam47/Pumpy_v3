import { pgTable, text, timestamp } from "drizzle-orm/pg-core";

export const walletRegistryTable = pgTable("wallet_registry", {
  walletId: text("wallet_id").primaryKey(),
  keypairPath: text("keypair_path"),
  status: text("status").notNull().default("enabled"),
  ownerPubkey: text("owner_pubkey"),
  lastActiveAt: timestamp("last_active_at", { withTimezone: true }),
  createdAt: timestamp("created_at", { withTimezone: true }).defaultNow(),
});

export type WalletRegistry = typeof walletRegistryTable.$inferSelect;
export type InsertWalletRegistry = typeof walletRegistryTable.$inferInsert;
