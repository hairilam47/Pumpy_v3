import { pgTable, text, uuid, doublePrecision, timestamp, integer, index } from "drizzle-orm/pg-core";
import { z } from "zod";

export const tradesTable = pgTable("trades", {
  id: text("id").primaryKey(),
  walletId: text("wallet_id"),
  mint: text("mint").notNull(),
  tokenName: text("token_name"),
  tokenSymbol: text("token_symbol"),
  side: text("side").notNull(),
  amountSol: doublePrecision("amount_sol").notNull(),
  price: doublePrecision("price"),
  status: text("status").notNull().default("Pending"),
  strategy: text("strategy").notNull().default("manual"),
  signature: text("signature"),
  pnlSol: doublePrecision("pnl_sol"),
  slippageBps: integer("slippage_bps").default(100),
  createdAt: timestamp("created_at").defaultNow(),
  executedAt: timestamp("executed_at"),
  clientOrderId: uuid("client_order_id"),
  traceId: text("trace_id"),
}, (table) => [
  index("trades_wallet_id_created_at_idx").on(table.walletId, table.createdAt),
  index("trades_mint_created_at_idx").on(table.mint, table.createdAt),
]);

export type InsertTrade = typeof tradesTable.$inferInsert;
export type Trade = typeof tradesTable.$inferSelect;
