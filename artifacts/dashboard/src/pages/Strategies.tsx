import { useListStrategies, useUpdateStrategy } from "@workspace/api-client-react";
import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Bot, Zap, TrendingUp, Brain, ToggleLeft, ToggleRight } from "lucide-react";
import { cn, formatSol, formatPercent } from "@/lib/utils";

const STRATEGY_ICONS: Record<string, React.ElementType> = {
  sniper: Zap,
  momentum: TrendingUp,
  ml: Brain,
};

const STRATEGY_DESCRIPTIONS: Record<string, string> = {
  sniper: "Identifies and buys newly launched tokens in the first minutes after creation, targeting early bonding curve stages before momentum builds.",
  momentum: "Detects tokens with strong price and volume momentum using ML signals and enters positions to capture trend continuation.",
};

export default function StrategiesPage() {
  const qc = useQueryClient();
  const { data: strategies, isLoading } = useListStrategies();
  const updateStrategy = useUpdateStrategy();
  const [editing, setEditing] = useState<string | null>(null);
  const [buyAmount, setBuyAmount] = useState<string>("");

  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const handleToggle = async (name: string, currentEnabled: boolean) => {
    try {
      await updateStrategy.mutateAsync({
        strategyName: name,
        data: { enabled: !currentEnabled },
      });
      qc.invalidateQueries({ queryKey: ["listStrategies"] });
    } catch (e) {
      console.error(e);
    }
  };

  const handleSaveBuyAmount = async (name: string) => {
    const amount = parseFloat(buyAmount);
    if (isNaN(amount) || amount <= 0) return;
    try {
      await updateStrategy.mutateAsync({
        strategyName: name,
        data: { buyAmountSol: amount },
      });
      qc.invalidateQueries({ queryKey: ["listStrategies"] });
      setEditing(null);
      setBuyAmount("");
    } catch (e) {
      console.error(e);
    }
  };

  if (isLoading) {
    return <div className="text-muted-foreground text-sm">Loading strategies...</div>;
  }

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-lg font-bold">Strategy Configurator</h1>
        <p className="text-sm text-muted-foreground mt-1">Enable, disable, and configure trading strategies in real time.</p>
      </div>

      <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
        {/* eslint-disable-next-line @typescript-eslint/no-explicit-any */}
        {(strategies as any[] | undefined)?.map((s) => {
          const Icon = STRATEGY_ICONS[s.name] ?? Bot;
          return (
            <div key={s.name} className={cn(
              "bg-card border rounded-xl p-5 transition-all",
              s.enabled ? "border-primary/40" : "border-border opacity-70"
            )}>
              {/* Header */}
              <div className="flex items-start justify-between mb-3">
                <div className="flex items-center gap-3">
                  <div className={cn("p-2 rounded-lg", s.enabled ? "bg-primary/10" : "bg-secondary")}>
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
                  className="transition-transform hover:scale-110"
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

              {/* Buy amount editor */}
              <div className="border-t border-border pt-3">
                {editing === s.name ? (
                  <div className="flex items-center gap-2">
                    <input
                      type="number"
                      className="flex-1 bg-secondary border border-border rounded px-2 py-1 text-sm tabular-nums focus:outline-none focus:ring-1 focus:ring-primary"
                      placeholder="SOL amount"
                      value={buyAmount}
                      onChange={(e) => setBuyAmount(e.target.value)}
                      step="0.01"
                      min="0.001"
                    />
                    <button
                      onClick={() => handleSaveBuyAmount(s.name)}
                      className="px-3 py-1 bg-primary text-primary-foreground rounded text-xs font-medium hover:opacity-90"
                    >
                      Save
                    </button>
                    <button
                      onClick={() => { setEditing(null); setBuyAmount(""); }}
                      className="px-3 py-1 bg-secondary rounded text-xs text-muted-foreground hover:text-foreground"
                    >
                      Cancel
                    </button>
                  </div>
                ) : (
                  <div className="flex items-center justify-between">
                    <div className="text-xs text-muted-foreground">
                      Buy amount: <span className="text-foreground tabular-nums font-medium">
                        {s.buyAmountSol ? formatSol(s.buyAmountSol) : "—"}
                      </span>
                    </div>
                    <button
                      onClick={() => { setEditing(s.name); setBuyAmount(String(s.buyAmountSol ?? "")); }}
                      className="text-xs text-primary hover:underline"
                    >
                      Edit
                    </button>
                  </div>
                )}
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}
