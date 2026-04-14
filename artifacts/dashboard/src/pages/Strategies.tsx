import { useListStrategies, useUpdateStrategy } from "@workspace/api-client-react";
import { useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { Bot, Zap, TrendingUp, Brain, ToggleLeft, ToggleRight, Save, X } from "lucide-react";
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

interface StrategyParams {
  buyAmountSol: string;
  slippageBps: string;
  takeProfitPct: string;
  stopLossPct: string;
  trailingStopPct: string;
  minLiquiditySol: string;
}

function defaultParams(s: { buyAmountSol?: number }): StrategyParams {
  return {
    buyAmountSol: s.buyAmountSol != null ? String(s.buyAmountSol) : "0.1",
    slippageBps: "100",
    takeProfitPct: "50",
    stopLossPct: "15",
    trailingStopPct: "10",
    minLiquiditySol: "5",
  };
}

interface ParamFieldProps {
  label: string;
  value: string;
  onChange: (v: string) => void;
  step?: string;
  min?: string;
  unit?: string;
}
function ParamField({ label, value, onChange, step = "1", min = "0", unit }: ParamFieldProps) {
  return (
    <div className="flex flex-col gap-1.5">
      <label className="text-xs text-muted-foreground">{label}</label>
      <div className="flex items-center gap-1.5">
        <input
          type="number"
          className="w-full bg-background border border-border rounded-lg px-3 py-2.5 sm:py-2 text-sm tabular-nums focus:outline-none focus:ring-1 focus:ring-primary min-h-[44px] sm:min-h-0"
          value={value}
          onChange={(e) => onChange(e.target.value)}
          step={step}
          min={min}
        />
        {unit && <span className="text-xs text-muted-foreground shrink-0 w-8">{unit}</span>}
      </div>
    </div>
  );
}

export default function StrategiesPage() {
  const qc = useQueryClient();
  const { data: strategies, isLoading } = useListStrategies();
  const updateStrategy = useUpdateStrategy();
  const [editing, setEditing] = useState<string | null>(null);
  const [params, setParams] = useState<StrategyParams>(defaultParams({}));
  const [saveError, setSaveError] = useState<string | null>(null);

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

  const openEditor = (s: { name: string; buyAmountSol?: number }) => {
    setEditing(s.name);
    setParams(defaultParams(s));
    setSaveError(null);
  };

  const handleSave = async (name: string) => {
    setSaveError(null);
    const buyAmountSol = parseFloat(params.buyAmountSol);
    const slippageBps = parseInt(params.slippageBps, 10);
    const takeProfitPct = parseFloat(params.takeProfitPct);
    const stopLossPct = parseFloat(params.stopLossPct);
    const trailingStopPct = parseFloat(params.trailingStopPct);
    const minLiquiditySol = parseFloat(params.minLiquiditySol);

    if (isNaN(buyAmountSol) || buyAmountSol <= 0) {
      setSaveError("Buy amount must be > 0");
      return;
    }
    try {
      await updateStrategy.mutateAsync({
        strategyName: name,
        data: {
          buyAmountSol,
          slippageBps: isNaN(slippageBps) ? undefined : slippageBps,
          takeProfitPct: isNaN(takeProfitPct) ? undefined : takeProfitPct,
          stopLossPct: isNaN(stopLossPct) ? undefined : stopLossPct,
          trailingStopPct: isNaN(trailingStopPct) ? undefined : trailingStopPct,
          minLiquiditySol: isNaN(minLiquiditySol) ? undefined : minLiquiditySol,
        },
      });
      qc.invalidateQueries({ queryKey: ["/api/bot/strategies"] });
      setEditing(null);
    } catch (e: unknown) {
      setSaveError(e instanceof Error ? e.message : "Save failed");
    }
  };

  if (isLoading) {
    return <div className="text-muted-foreground text-sm">Loading strategies...</div>;
  }

  return (
    <div className="space-y-4 sm:space-y-6">
      <div>
        <h1 className="text-lg font-bold">Strategy Configurator</h1>
        <p className="text-sm text-muted-foreground mt-1">Enable, disable, and configure trading strategies in real time.</p>
      </div>

      <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
        {(strategies as { name: string; enabled: boolean; tradesExecuted: number; winRate: number; totalPnlSol: number; buyAmountSol?: number }[] | undefined)?.map((s) => {
          const Icon = STRATEGY_ICONS[s.name] ?? Bot;
          const isEditingThis = editing === s.name;
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

              {/* Config section */}
              <div className="border-t border-border pt-3">
                {isEditingThis ? (
                  <div className="space-y-3">
                    <div className="text-xs font-semibold text-foreground mb-2">Parameters</div>
                    <div className="grid grid-cols-2 gap-3">
                      <ParamField
                        label="Buy Amount"
                        value={params.buyAmountSol}
                        onChange={(v) => setParams((p) => ({ ...p, buyAmountSol: v }))}
                        step="0.01"
                        min="0.001"
                        unit="SOL"
                      />
                      <ParamField
                        label="Slippage"
                        value={params.slippageBps}
                        onChange={(v) => setParams((p) => ({ ...p, slippageBps: v }))}
                        step="10"
                        min="0"
                        unit="bps"
                      />
                      <ParamField
                        label="Take Profit"
                        value={params.takeProfitPct}
                        onChange={(v) => setParams((p) => ({ ...p, takeProfitPct: v }))}
                        step="1"
                        min="1"
                        unit="%"
                      />
                      <ParamField
                        label="Stop Loss"
                        value={params.stopLossPct}
                        onChange={(v) => setParams((p) => ({ ...p, stopLossPct: v }))}
                        step="1"
                        min="1"
                        unit="%"
                      />
                      <ParamField
                        label="Trailing Stop"
                        value={params.trailingStopPct}
                        onChange={(v) => setParams((p) => ({ ...p, trailingStopPct: v }))}
                        step="1"
                        min="0"
                        unit="%"
                      />
                      <ParamField
                        label="Min Liquidity"
                        value={params.minLiquiditySol}
                        onChange={(v) => setParams((p) => ({ ...p, minLiquiditySol: v }))}
                        step="0.5"
                        min="0"
                        unit="SOL"
                      />
                    </div>
                    {saveError && (
                      <p className="text-xs text-red-400">{saveError}</p>
                    )}
                    <div className="flex gap-2 pt-1">
                      <button
                        onClick={() => handleSave(s.name)}
                        disabled={updateStrategy.isPending}
                        className="flex items-center gap-1.5 px-4 py-2.5 sm:py-1.5 bg-primary text-primary-foreground rounded-lg text-xs font-semibold hover:opacity-90 disabled:opacity-50 min-h-[44px] sm:min-h-0"
                      >
                        <Save className="w-3.5 h-3.5" />
                        Save
                      </button>
                      <button
                        onClick={() => { setEditing(null); setSaveError(null); }}
                        className="flex items-center gap-1.5 px-4 py-2.5 sm:py-1.5 bg-secondary rounded-lg text-xs text-muted-foreground hover:text-foreground min-h-[44px] sm:min-h-0"
                      >
                        <X className="w-3.5 h-3.5" />
                        Cancel
                      </button>
                    </div>
                  </div>
                ) : (
                  <div className="flex items-center justify-between">
                    <div className="flex gap-4 text-xs text-muted-foreground">
                      <span>Buy: <span className="text-foreground font-medium tabular-nums">
                        {s.buyAmountSol != null ? formatSol(s.buyAmountSol) : "—"}
                      </span></span>
                    </div>
                    <button
                      onClick={() => openEditor(s)}
                      className="text-xs text-primary hover:underline py-2 px-1 min-h-[44px] flex items-center"
                    >
                      Configure
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
