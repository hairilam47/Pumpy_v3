import { pgTable, text, doublePrecision, timestamp, integer } from "drizzle-orm/pg-core";
import { z } from "zod";

export const tradesTable = pgTable("trades", {
  id: text("id").primaryKey(),
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
});

export type InsertTrade = typeof tradesTable.$inferInsert;
export type Trade = typeof tradesTable.$inferSelect;
