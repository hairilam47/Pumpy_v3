import { pgTable, text, boolean, doublePrecision, integer, timestamp } from "drizzle-orm/pg-core";

export const strategiesTable = pgTable("strategies", {
  name: text("name").primaryKey(),
  enabled: boolean("enabled").notNull().default(true),
  buyAmountSol: doublePrecision("buy_amount_sol").default(0.05),
  slippageBps: integer("slippage_bps").default(100),
  tradesExecuted: integer("trades_executed").notNull().default(0),
  tradesWon: integer("trades_won").notNull().default(0),
  totalPnlSol: doublePrecision("total_pnl_sol").notNull().default(0),
  updatedAt: timestamp("updated_at").defaultNow(),
});

export type InsertStrategy = typeof strategiesTable.$inferInsert;
export type Strategy = typeof strategiesTable.$inferSelect;
