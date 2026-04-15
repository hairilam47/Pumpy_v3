import { pgTable, text, integer, timestamp, index } from "drizzle-orm/pg-core";

export const walletAlertsTable = pgTable("wallet_alerts", {
  walletId: text("wallet_id").notNull(),
  errorType: text("error_type").notNull(),
  count: integer("count").notNull().default(0),
  lastAt: timestamp("last_at", { withTimezone: true }).defaultNow(),
  autoPausedAt: timestamp("auto_paused_at", { withTimezone: true }),
  createdAt: timestamp("created_at", { withTimezone: true }).defaultNow(),
}, (table) => [
  index("wallet_alerts_wallet_id_created_at_idx").on(table.walletId, table.createdAt),
]);

export type WalletAlert = typeof walletAlertsTable.$inferSelect;
export type InsertWalletAlert = typeof walletAlertsTable.$inferInsert;
