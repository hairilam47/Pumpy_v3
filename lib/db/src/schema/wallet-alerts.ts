import { pgTable, text, integer, timestamp } from "drizzle-orm/pg-core";

export const walletAlertsTable = pgTable("wallet_alerts", {
  walletId: text("wallet_id").notNull(),
  errorType: text("error_type").notNull(),
  count: integer("count").notNull().default(0),
  lastAt: timestamp("last_at", { withTimezone: true }).defaultNow(),
  autoPausedAt: timestamp("auto_paused_at", { withTimezone: true }),
});

export type WalletAlert = typeof walletAlertsTable.$inferSelect;
export type InsertWalletAlert = typeof walletAlertsTable.$inferInsert;
