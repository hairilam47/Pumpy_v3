import { useListStrategies, useUpdateStrategy } from "@workspace/api-client-react";
import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Bot, Zap, TrendingUp, Brain, ToggleLeft, ToggleRight,
  Shield, ChevronRight,
} from "lucide-react";
import { cn, formatPercent } from "@/lib/utils";
import { Link } from "wouter";

const STRATEGY_ICONS: Record<string, React.ElementType> = {
  sniper: Zap,
  momentum: TrendingUp,
  ml: Brain,
};

const STRATEGY_DESCRIPTIONS: Record<string, string> = {
  sniper: "Identifies and buys newly launched tokens in the first minutes after creation, targeting early bonding curve stages before momentum builds.",
  momentum: "Detects tokens with strong price and volume momentum using ML signals and enters positions to capture trend continuation.",
};

const PRESET_META: Record<string, { label: string; color: string; detail: string }> = {
  conservative: {
    label: "Conservative",
    color: "bg-blue-500/15 text-blue-400 border-blue-500/30",
    detail: "0.05 SOL/trade · 5% stop · 20% target",
  },
  balanced: {
    label: "Balanced",
    color: "bg-primary/15 text-primary border-primary/30",
    detail: "0.15 SOL/trade · 10% stop · 50% target",
  },
  aggressive: {
    label: "Aggressive",
    color: "bg-orange-500/15 text-orange-400 border-orange-500/30",
    detail: "0.5 SOL/trade · 20% stop · 100% target",
  },
};

function useActivePreset() {
  return useQuery<{ preset: string }>({
    queryKey: ["activePreset"],
    queryFn: async () => {
      const res = await fetch("/api/wallets/wallet_001/config");
      if (!res.ok) return { preset: "balanced" };
      const data = (await res.json()) as { strategyPreset?: string };
      return { preset: data.strategyPreset ?? "balanced" };
    },
    staleTime: 10_000,
    refetchInterval: 30_000,
  });
}

export default function StrategiesPage() {
  const qc = useQueryClient();
  const { data: strategies, isLoading } = useListStrategies();
  const updateStrategy = useUpdateStrategy();
  const { data: presetData } = useActivePreset();

  const activePreset = presetData?.preset ?? "balanced";
  const presetMeta = PRESET_META[activePreset] ?? PRESET_META.balanced;

  const handleToggle = async (name: string, currentEnabled: boolean) => {
    try {
      await updateStrategy.mutateAsync({
        strategyName: name,
        data: { enabled: !currentEnabled },
      });
      qc.invalidateQueries({ queryKey: ["/api/bot/strategies"] });
    } catch (e) {
      console.error(e);
    }
  };

  if (isLoading) {
    return <div className="text-muted-foreground text-sm">Loading strategies...</div>;
  }

  return (
    <div className="space-y-4 sm:space-y-6">
      <div>
        <h1 className="text-lg font-bold">Strategy Configurator</h1>
        <p className="text-sm text-muted-foreground mt-1">
          Enable or disable trading strategies. Risk parameters are controlled by the active preset.
        </p>
      </div>

      {/* Active Preset Banner */}
      <div className={cn(
        "rounded-xl border px-4 py-3 flex items-center justify-between gap-4",
        presetMeta.color,
      )}>
        <div className="flex items-center gap-3">
          <Shield className="w-5 h-5 flex-shrink-0" />
          <div>
            <div className="text-sm font-semibold">
              Active preset: {presetMeta.label}
            </div>
            <div className="text-xs opacity-75 mt-0.5">{presetMeta.detail}</div>
          </div>
        </div>
        <Link
          href="/settings"
          className="flex items-center gap-1 text-xs font-medium opacity-80 hover:opacity-100 transition-opacity shrink-0"
        >
          Change
          <ChevronRight className="w-3.5 h-3.5" />
        </Link>
      </div>

      <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
        {(strategies as { name: string; enabled: boolean; tradesExecuted: number; winRate: number; totalPnlSol: number; buyAmountSol?: number }[] | undefined)?.map((s) => {
          const Icon = STRATEGY_ICONS[s.name] ?? Bot;
          return (
            <div key={s.name} className={cn(
              "bg-card border rounded-xl p-4 sm:p-5 transition-all",
              s.enabled ? "border-primary/40" : "border-border opacity-80"
            )}>
              {/* Header */}
              <div className="flex items-start justify-between mb-3">
                <div className="flex items-center gap-3">
                  <div className={cn("p-2 rounded-lg flex-shrink-0", s.enabled ? "bg-primary/10" : "bg-secondary")}>
                    <Icon className={cn("w-5 h-5", s.enabled ? "text-primary" : "text-muted-foreground")} />
                  </div>
                  <div>
                    <h3 className="font-semibold capitalize">{s.name}</h3>
                    <span className={cn("text-xs", s.enabled ? "text-primary" : "text-muted-foreground")}>
                      {s.enabled ? "Active" : "Disabled"}
                    </span>
                  </div>
                </div>
                <button
                  onClick={() => handleToggle(s.name, s.enabled)}
                  disabled={updateStrategy.isPending}
                  className="transition-transform hover:scale-110 active:scale-95 p-1 min-h-[44px] min-w-[44px] flex items-center justify-center"
                  title={s.enabled ? "Disable strategy" : "Enable strategy"}
                >
                  {s.enabled ? (
                    <ToggleRight className="w-8 h-8 text-primary" />
                  ) : (
                    <ToggleLeft className="w-8 h-8 text-muted-foreground" />
                  )}
                </button>
              </div>

              {/* Description */}
              <p className="text-xs text-muted-foreground mb-4 leading-relaxed">
                {STRATEGY_DESCRIPTIONS[s.name] ?? "Advanced trading strategy."}
              </p>

              {/* Stats */}
              <div className="grid grid-cols-3 gap-3 mb-4">
                <div className="text-center">
                  <div className="text-lg font-bold tabular-nums">{s.tradesExecuted}</div>
                  <div className="text-xs text-muted-foreground">Trades</div>
                </div>
                <div className="text-center">
                  <div className={cn("text-lg font-bold tabular-nums",
                    s.winRate > 50 ? "text-green-400" : s.winRate > 0 ? "text-amber-400" : "text-muted-foreground"
                  )}>
                    {formatPercent(s.winRate)}
                  </div>
                  <div className="text-xs text-muted-foreground">Win Rate</div>
                </div>
                <div className="text-center">
                  <div className={cn("text-lg font-bold tabular-nums",
                    s.totalPnlSol >= 0 ? "text-green-400" : "text-red-400"
                  )}>
                    {s.totalPnlSol >= 0 ? "+" : ""}{s.totalPnlSol.toFixed(3)}
                  </div>
                  <div className="text-xs text-muted-foreground">PnL (SOL)</div>
                </div>
              </div>

              {/* Preset indicator at bottom */}
              <div className="border-t border-border pt-3">
                <div className="flex items-center justify-between">
                  <span className="text-xs text-muted-foreground">Risk profile</span>
                  <span className={cn(
                    "inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-xs font-medium border",
                    presetMeta.color,
                  )}>
                    <Shield className="w-2.5 h-2.5" />
                    {presetMeta.label}
                  </span>
                </div>
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
