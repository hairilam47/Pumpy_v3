import { pgTable, text, timestamp } from "drizzle-orm/pg-core";

export const botConfigTable = pgTable("bot_config", {
  key: text("key").primaryKey(),
  value: text("value").notNull(),
  updatedAt: timestamp("updated_at").defaultNow(),
});

export type BotConfig = typeof botConfigTable.$inferSelect;
export type InsertBotConfig = typeof botConfigTable.$inferInsert;
