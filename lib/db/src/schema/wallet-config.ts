import { pgTable, text, doublePrecision, timestamp } from "drizzle-orm/pg-core";

export const walletConfigTable = pgTable("wallet_config", {
  walletId: text("wallet_id").primaryKey(),
  riskPerTradeSol: doublePrecision("risk_per_trade_sol").notNull().default(0.1),
  dailyLossLimitSol: doublePrecision("daily_loss_limit_sol").notNull().default(1.0),
  strategyPreset: text("strategy_preset").notNull().default("balanced"),
  status: text("status").notNull().default("enabled"),
  ownerPubkey: text("owner_pubkey"),
  createdAt: timestamp("created_at", { withTimezone: true }).defaultNow(),
  updatedAt: timestamp("updated_at", { withTimezone: true }).defaultNow(),
});

export type WalletConfig = typeof walletConfigTable.$inferSelect;
export type InsertWalletConfig = typeof walletConfigTable.$inferInsert;
