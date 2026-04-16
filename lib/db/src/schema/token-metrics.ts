import { pgTable, text, doublePrecision, timestamp, integer, index, serial } from "drizzle-orm/pg-core";

export const tokenMetricsTable = pgTable("token_metrics", {
  id: serial("id").primaryKey(),
  mint: text("mint").notNull(),
  price: doublePrecision("price").notNull(),
  liquiditySol: doublePrecision("liquidity_sol"),
  marketCapSol: doublePrecision("market_cap_sol"),
  volume24hSol: doublePrecision("volume_24h_sol"),
  holderCount: integer("holder_count"),
  bondingCurveProgress: doublePrecision("bonding_curve_progress"),
  recordedAt: timestamp("recorded_at").defaultNow().notNull(),
}, (table) => [
  index("token_metrics_mint_recorded_at_idx").on(table.mint, table.recordedAt),
  index("token_metrics_recorded_at_idx").on(table.recordedAt),
]);

export type InsertTokenMetric = typeof tokenMetricsTable.$inferInsert;
export type TokenMetric = typeof tokenMetricsTable.$inferSelect;
