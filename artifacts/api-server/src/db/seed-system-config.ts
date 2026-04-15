/**
 * Seed script for Layer B system_config table.
 * Run once after the table is created; safe to re-run (ON CONFLICT DO NOTHING).
 *
 * Usage:
 *   pnpm --filter @workspace/api-server exec tsx src/db/seed-system-config.ts
 */

import { db, systemConfigTable } from "@workspace/db";

const SEED: Array<{
  key: string;
  value: string;
  description: string;
}> = [
  // ── Risk caps ────────────────────────────────────────────────────────────
  { key: "max_position_size_sol", value: "1.0", description: "Maximum SOL per individual trade position" },
  { key: "max_portfolio_exposure_sol", value: "5.0", description: "Maximum total SOL exposure across all open positions" },
  { key: "max_daily_loss_sol", value: "2.0", description: "Maximum SOL loss allowed in a 24-hour window before halting" },
  { key: "max_slippage_bps", value: "1000", description: "Maximum allowed slippage in basis points (1000 = 10%)" },
  { key: "max_sandwich_risk_score", value: "70", description: "Maximum sandwich attack risk score (0-100) before rejecting trade" },

  // ── MEV / Jito ───────────────────────────────────────────────────────────
  { key: "jito_tip_lamports", value: "10000", description: "Default Jito bundle tip in lamports" },
  { key: "jito_bundle_timeout_ms", value: "5000", description: "Jito bundle submission timeout in milliseconds" },
  { key: "mev_protection_enabled", value: "true", description: "Whether MEV protection via Jito is enabled by default" },

  // ── Token discovery thresholds ───────────────────────────────────────────
  { key: "min_liquidity_sol", value: "5.0", description: "Minimum liquidity (SOL) required for a token to be considered for sniping" },
  { key: "min_market_cap_usd", value: "1000", description: "Minimum market cap USD for token discovery filter" },
  { key: "max_token_age_seconds", value: "30", description: "Maximum age in seconds for auto-snipe eligibility" },

  // ── Strategy availability flags ──────────────────────────────────────────
  { key: "strategy_sniper_enabled", value: "true", description: "Whether the sniper strategy is available for activation" },
  { key: "strategy_momentum_enabled", value: "true", description: "Whether the momentum strategy is available for activation" },
  { key: "strategy_scalp_enabled", value: "false", description: "Whether the scalp strategy is available for activation" },

  // ── Order execution ──────────────────────────────────────────────────────
  { key: "default_snipe_amount_sol", value: "0.1", description: "Default SOL amount for auto-snipe orders" },
  { key: "max_retries", value: "3", description: "Maximum order retry attempts before marking as failed" },
  { key: "retry_delay_ms", value: "1000", description: "Delay in milliseconds between order retries" },
];

async function seed() {
  console.log(`Seeding ${SEED.length} system_config rows…`);
  let inserted = 0;
  for (const row of SEED) {
    const result = await db
      .insert(systemConfigTable)
      .values({
        key: row.key,
        value: row.value,
        description: row.description,
        updatedBy: "system:seed",
        updatedAt: new Date(),
      })
      .onConflictDoNothing()
      .returning();
    if (result.length > 0) inserted++;
  }
  console.log(`Done. ${inserted} rows inserted (${SEED.length - inserted} already existed).`);
  process.exit(0);
}

seed().catch((err) => {
  console.error("Seed failed:", err);
  process.exit(1);
});
