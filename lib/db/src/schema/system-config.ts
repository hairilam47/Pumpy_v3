import { pgTable, text, integer, timestamp } from "drizzle-orm/pg-core";

export const systemConfigTable = pgTable("system_config", {
  key: text("key").primaryKey(),
  value: text("value").notNull(),
  version: integer("version").notNull().default(1),
  description: text("description"),
  updatedBy: text("updated_by"),
  updatedAt: timestamp("updated_at", { withTimezone: true }).defaultNow(),
});

export type SystemConfig = typeof systemConfigTable.$inferSelect;
export type InsertSystemConfig = typeof systemConfigTable.$inferInsert;
