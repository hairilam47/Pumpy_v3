import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Shield, Loader2, AlertTriangle, CheckCircle2, Zap, TrendingUp, KeyRound, X, FlaskConical, BarChart2 } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { cn } from "@/lib/utils";
import { useToast } from "@/hooks/use-toast";
import { useAdminKey, getValidAdminKey } from "@/hooks/use-admin-key";
import {
  AreaChart,
  Area,
  XAxis,
  YAxis,
  Tooltip,
  ResponsiveContainer,
  CartesianGrid,
} from "recharts";

interface WalletEntry {
  walletId: string;
  status: string;
}

interface WalletConfig {
  walletId: string;
  strategyPreset: string;
  status: string;
  riskPerTradeSol: number;
  dailyLossLimitSol: number;
}

const PRESETS = [
  {
    id: "conservative",
    label: "Conservative",
    icon: Shield,
    risk: "Low risk",
    params: { buy: "0.05 SOL", stop: "5%", target: "20%" },
    description: "Small positions with tight stops. Best for volatile markets or when starting out.",
    accent: "border-blue-500/40 bg-blue-500/5",
    activeAccent: "border-blue-500 bg-blue-500/15 ring-1 ring-blue-500/30",
    labelColor: "text-blue-400",
    riskColor: "bg-blue-500/15 text-blue-400",
  },
  {
    id: "balanced",
    label: "Balanced",
    icon: TrendingUp,
    risk: "Medium risk",
    params: { buy: "0.15 SOL", stop: "10%", target: "50%" },
    description: "Moderate position sizes with a balanced risk/reward ratio. Recommended default.",
    accent: "border-primary/40 bg-primary/5",
    activeAccent: "border-primary bg-primary/15 ring-1 ring-primary/30",
    labelColor: "text-primary",
    riskColor: "bg-primary/15 text-primary",
  },
  {
    id: "aggressive",
    label: "Aggressive",
    icon: Zap,
    risk: "High risk",
    params: { buy: "0.5 SOL", stop: "20%", target: "100%" },
    description: "Large positions targeting high returns. Higher drawdown risk — use with caution.",
    accent: "border-orange-500/40 bg-orange-500/5",
    activeAccent: "border-orange-500 bg-orange-500/15 ring-1 ring-orange-500/30",
    labelColor: "text-orange-400",
    riskColor: "bg-orange-500/15 text-orange-400",
  },
] as const;

type PresetId = typeof PRESETS[number]["id"];

// Preset quick-fill values for the backtest parameter overrides
const PRESET_DEFAULTS: Record<PresetId, { buy: number; stop: number; take: number }> = {
  conservative: { buy: 0.05, stop: 5, take: 20 },
  balanced:     { buy: 0.15, stop: 10, take: 50 },
  aggressive:   { buy: 0.5,  stop: 20, take: 100 },
};

// Strategy engine strategies available for backtesting
const STRATEGIES = [
  {
    id: "sniper",
    label: "Sniper",
    description: "Enters early when bonding curve progress < 30% with upward momentum.",
    accent: "border-blue-500/40 bg-blue-500/5",
    activeAccent: "border-blue-500 bg-blue-500/20 ring-1 ring-blue-500/40",
    labelColor: "text-blue-400",
  },
  {
    id: "momentum",
    label: "Momentum",
    description: "Enters when 5-bar MA crosses above 20-bar MA.",
    accent: "border-orange-500/40 bg-orange-500/5",
    activeAccent: "border-orange-500 bg-orange-500/20 ring-1 ring-orange-500/40",
    labelColor: "text-orange-400",
  },
] as const;

type StrategyId = typeof STRATEGIES[number]["id"];

// Compute peak-to-trough max drawdown in SOL from an equity curve
function peakToTroughDrawdownSol(curve: number[]): number {
  let peak = -Infinity;
  let maxDD = 0;
  for (const v of curve) {
    if (v > peak) peak = v;
    const dd = peak - v;
    if (dd > maxDD) maxDD = dd;
  }
  return maxDD;
}

interface BacktestResult {
  strategy: string;
  days: number;
  initial_sol: number;
  final_sol: number;
  simulated_pnl_sol: number;
  total_return_pct: number;
  total_trades: number;
  wins: number;
  win_rate: number;
  sharpe_ratio: number;
  max_drawdown_pct: number;
  volatility: number;
  equity_curve: number[];
  data_source: string;
}

function useWallets() {
  return useQuery<WalletEntry[]>({
    queryKey: ["wallets"],
    queryFn: async () => {
      const res = await fetch("/api/wallets");
      if (!res.ok) throw new Error("Failed to fetch wallets");
      return res.json() as Promise<WalletEntry[]>;
    },
    refetchInterval: 15_000,
  });
}

function useWalletConfig(walletId: string) {
  return useQuery<WalletConfig>({
    queryKey: ["walletConfig", walletId],
    queryFn: async () => {
      const res = await fetch(`/api/wallets/${walletId}/config`);
      if (!res.ok) return { walletId, strategyPreset: "balanced", status: "enabled", riskPerTradeSol: 0.15, dailyLossLimitSol: 1 };
      return res.json() as Promise<WalletConfig>;
    },
    staleTime: 10_000,
    refetchInterval: 30_000,
  });
}

function WalletPresetCard({ wallet }: { wallet: WalletEntry }) {
  const { toast } = useToast();
  const qc = useQueryClient();
  const { data: config, isLoading } = useWalletConfig(wallet.walletId);
  const [pendingPreset, setPendingPreset] = useState<PresetId | null>(null);
  const { adminKey, setAdminKey, rememberAdminKey, clearAdminKey } = useAdminKey();
  const [rememberKey, setRememberKey] = useState(() => adminKey.length > 0);

  const activePreset = (config?.strategyPreset ?? "balanced") as PresetId;

  const changePreset = useMutation({
    mutationFn: async ({ preset, key }: { preset: PresetId; key: string }) => {
      const res = await fetch("/api/strategy/preset", {
        method: "PUT",
        headers: {
          "Content-Type": "application/json",
          "X-Admin-Key": key,
        },
        body: JSON.stringify({ preset, wallet_id: wallet.walletId }),
      });
      if (!res.ok) {
        const err = (await res.json()) as { error: string };
        throw new Error(err.error ?? "Failed to change preset");
      }
      return res.json();
    },
    onSuccess: (_data, { preset, key }) => {
      toast({ title: `Preset changed to ${preset} for ${wallet.walletId}` });
      if (pendingPreset) {
        if (rememberKey) { rememberAdminKey(key); } else { clearAdminKey(); }
      }
      setPendingPreset(null);
      void qc.invalidateQueries({ queryKey: ["walletConfig", wallet.walletId] });
    },
    onError: (err: Error) => {
      toast({ title: "Failed to change preset", description: err.message, variant: "destructive" });
      clearAdminKey();
      setRememberKey(false);
    },
  });

  return (
    <Card>
      <CardHeader className="pb-3">
        <CardTitle className="text-sm flex items-center gap-2 font-mono">
          <Shield className="w-4 h-4 text-primary flex-shrink-0" />
          {wallet.walletId}
          {wallet.status !== "enabled" && (
            <span className={cn(
              "ml-auto text-xs px-2 py-0.5 rounded-full font-sans font-medium",
              wallet.status === "paused" ? "bg-yellow-500/15 text-yellow-400" : "bg-red-500/15 text-red-400"
            )}>
              {wallet.status}
            </span>
          )}
        </CardTitle>
      </CardHeader>
      <CardContent>
        {isLoading ? (
          <div className="flex items-center gap-2 text-xs text-muted-foreground">
            <Loader2 className="w-3.5 h-3.5 animate-spin" />
            Loading config…
          </div>
        ) : (
          <>
          <div className="grid grid-cols-1 sm:grid-cols-3 gap-3">
            {PRESETS.map((preset) => {
              const Icon = preset.icon;
              const isActive = activePreset === preset.id;
              const isPending = pendingPreset === preset.id;

              return (
                <button
                  key={preset.id}
                  onClick={() => {
                    if (!isActive && !pendingPreset) {
                      const key = getValidAdminKey();
                      if (key) {
                        changePreset.mutate({ preset: preset.id, key });
                      } else {
                        setPendingPreset(preset.id);
                      }
                    }
                  }}
                  disabled={changePreset.isPending || !!pendingPreset}
                  className={cn(
                    "relative rounded-xl border p-4 text-left transition-all focus:outline-none focus-visible:ring-2 focus-visible:ring-primary/50",
                    isActive ? preset.activeAccent : cn(preset.accent, !pendingPreset && "hover:brightness-110 cursor-pointer"),
                    isActive && "cursor-default",
                    isPending && "ring-2 ring-primary/60",
                    !!pendingPreset && !isPending && !isActive && "opacity-50",
                  )}
                >
                  {isActive && (
                    <CheckCircle2 className="absolute top-2.5 right-2.5 w-4 h-4 text-primary" />
                  )}
                  <div className="flex items-center gap-2 mb-2">
                    <Icon className={cn("w-4 h-4", preset.labelColor)} />
                    <span className={cn("text-sm font-semibold", preset.labelColor)}>{preset.label}</span>
                  </div>
                  <span className={cn("inline-block text-xs px-1.5 py-0.5 rounded-full font-medium mb-2", preset.riskColor)}>
                    {preset.risk}
                  </span>
                  <div className="space-y-0.5 mb-3">
                    <div className="flex justify-between text-xs">
                      <span className="text-muted-foreground">Position size</span>
                      <span className="font-medium tabular-nums">{preset.params.buy}</span>
                    </div>
                    <div className="flex justify-between text-xs">
                      <span className="text-muted-foreground">Stop loss</span>
                      <span className="font-medium tabular-nums text-red-400">{preset.params.stop}</span>
                    </div>
                    <div className="flex justify-between text-xs">
                      <span className="text-muted-foreground">Take profit</span>
                      <span className="font-medium tabular-nums text-green-400">{preset.params.target}</span>
                    </div>
                  </div>
                  <p className="text-xs text-muted-foreground leading-relaxed">{preset.description}</p>
                  {!isActive && (
                    <div className="mt-3">
                      {isPending && changePreset.isPending ? (
                        <div className="flex items-center gap-1 text-xs text-muted-foreground">
                          <Loader2 className="w-3 h-3 animate-spin" />
                          Applying…
                        </div>
                      ) : (
                        <span className={cn("text-xs font-medium", preset.labelColor)}>
                          {isPending ? "Selected — enter key below" : "Select →"}
                        </span>
                      )}
                    </div>
                  )}
                  {isActive && (
                    <div className="mt-3 text-xs font-medium text-primary">Active preset</div>
                  )}
                </button>
              );
            })}
          </div>

          {changePreset.isPending && !pendingPreset && (
            <div className="mt-3 flex items-center gap-2 text-xs text-muted-foreground rounded-lg bg-primary/5 px-3 py-2">
              <Loader2 className="w-3.5 h-3.5 animate-spin flex-shrink-0 text-primary" />
              Applying with remembered key…
            </div>
          )}

          {pendingPreset && (
            <div className="mt-3 rounded-lg border border-primary/30 bg-primary/5 px-3 py-2 space-y-2">
              <p className="text-xs text-muted-foreground">
                Changing to{" "}
                <span className="font-semibold text-foreground capitalize">{pendingPreset}</span>{" "}
                preset requires admin authorization.
              </p>
              <div className="flex items-center gap-2">
                <KeyRound className="w-4 h-4 text-primary flex-shrink-0" />
                <Input
                  type="password"
                  placeholder="Enter admin key…"
                  value={adminKey}
                  onChange={(e) => setAdminKey(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" && adminKey) changePreset.mutate({ preset: pendingPreset, key: adminKey });
                    if (e.key === "Escape") { setPendingPreset(null); clearAdminKey(); setRememberKey(false); }
                  }}
                  className="h-7 text-xs flex-1"
                  autoFocus
                />
                <Button
                  size="sm"
                  className="h-7 text-xs"
                  onClick={() => changePreset.mutate({ preset: pendingPreset, key: adminKey })}
                  disabled={!adminKey || changePreset.isPending}
                >
                  {changePreset.isPending ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : "Apply"}
                </Button>
                <Button
                  size="sm"
                  variant="ghost"
                  className="h-7 w-7 p-0"
                  onClick={() => { setPendingPreset(null); clearAdminKey(); setRememberKey(false); }}
                >
                  <X className="w-3.5 h-3.5" />
                </Button>
              </div>
              <Label className="flex items-center gap-2 text-xs text-muted-foreground cursor-pointer font-normal">
                <Checkbox
                  id={`remember-key-strategies-${wallet.walletId}`}
                  checked={rememberKey}
                  onCheckedChange={(c) => setRememberKey(c === true)}
                />
                Remember for 1 hour
              </Label>
            </div>
          )}
          </>
        )}
      </CardContent>
    </Card>
  );
}

// ─── Metric tile ──────────────────────────────────────────────────────────────
function MetricTile({
  label,
  value,
  sub,
  positive,
}: {
  label: string;
  value: string;
  sub?: string;
  positive?: boolean;
}) {
  return (
    <div className="rounded-lg border border-border/50 bg-muted/20 px-3 py-2.5">
      <p className="text-[11px] text-muted-foreground mb-0.5">{label}</p>
      <p className={cn(
        "text-base font-bold tabular-nums",
        positive === true && "text-green-400",
        positive === false && "text-red-400",
      )}>
        {value}
      </p>
      {sub && <p className="text-[10px] text-muted-foreground mt-0.5">{sub}</p>}
    </div>
  );
}

// ─── Equity curve chart ───────────────────────────────────────────────────────
function EquityCurveChart({ curve, initialSol }: { curve: number[]; initialSol: number }) {
  const data = curve.map((sol, i) => ({ i, sol }));
  const isPositive = curve[curve.length - 1] >= initialSol;

  return (
    <ResponsiveContainer width="100%" height={120}>
      <AreaChart data={data} margin={{ top: 4, right: 4, left: 0, bottom: 0 }}>
        <defs>
          <linearGradient id="eq-grad" x1="0" y1="0" x2="0" y2="1">
            <stop offset="5%" stopColor={isPositive ? "#22c55e" : "#ef4444"} stopOpacity={0.3} />
            <stop offset="95%" stopColor={isPositive ? "#22c55e" : "#ef4444"} stopOpacity={0} />
          </linearGradient>
        </defs>
        <CartesianGrid strokeDasharray="3 3" stroke="rgba(255,255,255,0.05)" />
        <XAxis dataKey="i" hide />
        <YAxis
          domain={["auto", "auto"]}
          tick={{ fontSize: 10, fill: "rgba(255,255,255,0.4)" }}
          width={42}
          tickFormatter={(v: number) => `${v.toFixed(2)}`}
        />
        <Tooltip
          contentStyle={{ background: "#0f172a", border: "1px solid rgba(255,255,255,0.1)", borderRadius: 6, fontSize: 11 }}
          labelFormatter={() => ""}
          formatter={(v: number) => [`${v.toFixed(4)} SOL`, "Equity"]}
        />
        <Area
          type="monotone"
          dataKey="sol"
          stroke={isPositive ? "#22c55e" : "#ef4444"}
          strokeWidth={1.5}
          fill="url(#eq-grad)"
          dot={false}
          isAnimationActive={false}
        />
      </AreaChart>
    </ResponsiveContainer>
  );
}

// ─── Backtest panel ───────────────────────────────────────────────────────────
function BacktestPanel() {
  const [selectedStrategy, setSelectedStrategy] = useState<StrategyId>("sniper");
  const [overrideBuy, setOverrideBuy] = useState("");
  const [overrideStop, setOverrideStop] = useState("");
  const [overrideTake, setOverrideTake] = useState("");
  const [days, setDays] = useState("7");
  const [result, setResult] = useState<BacktestResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  const runBacktest = useMutation({
    mutationFn: async () => {
      const body: Record<string, unknown> = {
        strategy_name: selectedStrategy,
        days: Number(days) || 7,
        initial_sol: 1,
      };
      const buyVal = parseFloat(overrideBuy);
      const stopVal = parseFloat(overrideStop);
      const takeVal = parseFloat(overrideTake);
      if (!isNaN(buyVal) && buyVal > 0)   body.buy_amount_sol  = buyVal;
      if (!isNaN(stopVal) && stopVal > 0)  body.stop_loss_pct   = stopVal;
      if (!isNaN(takeVal) && takeVal > 0)  body.take_profit_pct = takeVal;

      const res = await fetch("/api/bot/backtest", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body),
      });
      if (!res.ok) {
        const err = (await res.json()) as { error?: string };
        throw new Error(err.error ?? `Server error ${res.status}`);
      }
      return res.json() as Promise<BacktestResult>;
    },
    onSuccess: (data) => {
      setResult(data);
      setError(null);
    },
    onError: (err: Error) => {
      setError(err.message);
      setResult(null);
    },
  });

  const isPositiveReturn = result ? result.total_return_pct >= 0 : true;

  // Compute accurate max drawdown from the equity curve (peak-to-trough)
  const maxDrawdownSol = result?.equity_curve?.length
    ? peakToTroughDrawdownSol(result.equity_curve)
    : null;

  return (
    <Card>
      <CardHeader className="pb-3">
        <CardTitle className="text-sm flex items-center gap-2">
          <FlaskConical className="w-4 h-4 text-purple-400" />
          Strategy Backtest
          <span className="ml-auto text-xs font-normal text-muted-foreground bg-muted/40 px-2 py-0.5 rounded-full">
            Read-only simulation
          </span>
        </CardTitle>
      </CardHeader>
      <CardContent className="space-y-4">
        <p className="text-xs text-muted-foreground">
          Simulate a strategy against recent price history without affecting live settings. Results are estimates based on available market data.
        </p>

        {/* Strategy selector */}
        <div className="space-y-1.5">
          <Label className="text-xs font-medium text-muted-foreground">Strategy</Label>
          <div className="grid grid-cols-2 gap-2">
            {STRATEGIES.map((s) => (
              <button
                key={s.id}
                onClick={() => {
                  setSelectedStrategy(s.id);
                  setResult(null);
                  setError(null);
                }}
                className={cn(
                  "rounded-lg border px-3 py-2.5 text-xs font-medium transition-all text-left",
                  selectedStrategy === s.id
                    ? cn(s.activeAccent, s.labelColor)
                    : cn(s.accent, "text-muted-foreground hover:brightness-110"),
                )}
              >
                <span className={cn("font-semibold", selectedStrategy === s.id ? s.labelColor : "text-foreground/80")}>
                  {s.label}
                </span>
                <div className="text-[10px] mt-0.5 font-normal opacity-70 leading-tight">{s.description}</div>
              </button>
            ))}
          </div>
        </div>

        {/* Preset quick-fill */}
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-[11px] text-muted-foreground">Load param defaults:</span>
          {PRESETS.map((p) => (
            <button
              key={p.id}
              onClick={() => {
                const d = PRESET_DEFAULTS[p.id];
                setOverrideBuy(String(d.buy));
                setOverrideStop(String(d.stop));
                setOverrideTake(String(d.take));
              }}
              className={cn(
                "text-[11px] px-2 py-0.5 rounded-full border transition-all",
                p.accent, p.labelColor, "hover:brightness-125",
              )}
            >
              {p.label}
            </button>
          ))}
          <button
            onClick={() => { setOverrideBuy(""); setOverrideStop(""); setOverrideTake(""); }}
            className="text-[11px] px-2 py-0.5 rounded-full border border-border/30 text-muted-foreground hover:brightness-110 transition-all"
          >
            Clear
          </button>
        </div>

        {/* Parameter overrides */}
        <div className="grid grid-cols-2 sm:grid-cols-4 gap-3">
          <div className="space-y-1">
            <Label className="text-[11px] text-muted-foreground">Days to simulate</Label>
            <Input
              type="number"
              min={1}
              max={90}
              value={days}
              onChange={(e) => {
                const v = e.target.value;
                const n = parseInt(v, 10);
                if (v === "") { setDays(""); return; }
                setDays(String(Math.min(90, Math.max(1, isNaN(n) ? 1 : n))));
              }}
              className="h-7 text-xs"
              placeholder="7"
            />
          </div>
          <div className="space-y-1">
            <Label className="text-[11px] text-muted-foreground">Buy size (SOL)</Label>
            <Input
              type="number"
              min={0.001}
              step={0.01}
              value={overrideBuy}
              onChange={(e) => setOverrideBuy(e.target.value)}
              className="h-7 text-xs"
              placeholder="engine default"
            />
          </div>
          <div className="space-y-1">
            <Label className="text-[11px] text-muted-foreground">Stop loss %</Label>
            <Input
              type="number"
              min={0.1}
              max={100}
              step={0.5}
              value={overrideStop}
              onChange={(e) => setOverrideStop(e.target.value)}
              className="h-7 text-xs"
              placeholder="engine default"
            />
          </div>
          <div className="space-y-1">
            <Label className="text-[11px] text-muted-foreground">Take profit %</Label>
            <Input
              type="number"
              min={0.1}
              max={1000}
              step={1}
              value={overrideTake}
              onChange={(e) => setOverrideTake(e.target.value)}
              className="h-7 text-xs"
              placeholder="engine default"
            />
          </div>
        </div>

        {/* Run button */}
        <Button
          onClick={() => runBacktest.mutate()}
          disabled={runBacktest.isPending}
          className="w-full sm:w-auto"
          size="sm"
        >
          {runBacktest.isPending ? (
            <>
              <Loader2 className="w-3.5 h-3.5 animate-spin mr-2" />
              Running simulation…
            </>
          ) : (
            <>
              <BarChart2 className="w-3.5 h-3.5 mr-2" />
              Run Backtest
            </>
          )}
        </Button>

        {/* Error */}
        {error && (
          <div className="flex items-start gap-2 rounded-lg bg-red-500/10 border border-red-500/20 px-3 py-2 text-xs text-red-400">
            <AlertTriangle className="w-3.5 h-3.5 flex-shrink-0 mt-0.5" />
            {error}
          </div>
        )}

        {/* Results */}
        {result && (
          <div className="space-y-3 pt-1">
            <div className="flex items-center gap-2 text-xs text-muted-foreground">
              <span className="font-medium text-foreground capitalize">{result.strategy}</span>
              strategy · {result.days}d · {result.total_trades} trades
              <span className={cn(
                "ml-auto text-[10px] px-1.5 py-0.5 rounded-full",
                result.data_source === "db" ? "bg-green-500/10 text-green-400" : "bg-yellow-500/10 text-yellow-400"
              )}>
                {result.data_source === "db" ? "DB data" : "In-memory data"}
              </span>
            </div>

            <div className="grid grid-cols-2 sm:grid-cols-3 gap-2">
              <MetricTile
                label="Total return"
                value={`${result.total_return_pct >= 0 ? "+" : ""}${result.total_return_pct.toFixed(2)}%`}
                sub={`${result.simulated_pnl_sol >= 0 ? "+" : ""}${result.simulated_pnl_sol.toFixed(4)} SOL`}
                positive={isPositiveReturn}
              />
              <MetricTile
                label="Win rate"
                value={`${result.win_rate.toFixed(1)}%`}
                sub={`${result.wins} / ${result.total_trades} wins`}
                positive={result.win_rate >= 50}
              />
              <MetricTile
                label="Sharpe ratio"
                value={result.sharpe_ratio.toFixed(2)}
                sub="annualised"
                positive={result.sharpe_ratio >= 1}
              />
              <MetricTile
                label="Max drawdown"
                value={`${result.max_drawdown_pct.toFixed(2)}%`}
                sub={maxDrawdownSol !== null ? `${maxDrawdownSol.toFixed(4)} SOL peak-to-trough` : undefined}
                positive={false}
              />
              <MetricTile
                label="Volatility"
                value={result.volatility.toFixed(4)}
                sub="std dev of trade PnL"
              />
            </div>

            {result.equity_curve && result.equity_curve.length > 1 && (
              <div className="rounded-lg border border-border/40 bg-muted/10 p-3">
                <p className="text-[11px] text-muted-foreground mb-2">Equity curve (SOL)</p>
                <EquityCurveChart curve={result.equity_curve} initialSol={result.initial_sol} />
              </div>
            )}
          </div>
        )}
      </CardContent>
    </Card>
  );
}

// ─── Page ─────────────────────────────────────────────────────────────────────
export default function StrategiesPage() {
  const { data: wallets, isLoading, error } = useWallets();

  return (
    <div className="space-y-4 sm:space-y-6">
      <div>
        <h1 className="text-lg font-bold">Trading Strategy Preset</h1>
        <p className="text-sm text-muted-foreground mt-1">
          Select a risk profile for each wallet. The preset controls position size, stop-loss, and take-profit targets across all strategies.
        </p>
      </div>

      {isLoading && (
        <div className="flex items-center gap-2 text-sm text-muted-foreground animate-pulse">
          <Loader2 className="w-4 h-4 animate-spin" />
          Loading wallets…
        </div>
      )}

      {error && (
        <div className="flex items-center gap-2 text-sm text-destructive">
          <AlertTriangle className="w-4 h-4" />
          Failed to load wallets — API server may be offline.
        </div>
      )}

      {wallets && wallets.length === 0 && (
        <Card>
          <CardContent className="py-8 text-center text-sm text-muted-foreground">
            No wallets registered. Register a wallet to configure its strategy preset.
          </CardContent>
        </Card>
      )}

      {wallets?.map((w) => (
        <WalletPresetCard key={w.walletId} wallet={w} />
      ))}

      <div className="rounded-lg bg-muted/30 border border-border/40 px-4 py-3 text-xs text-muted-foreground space-y-1">
        <p className="font-medium text-foreground/70">About presets</p>
        <p>Presets apply immediately to new signals. Open positions are not affected until their own stop or target is hit.</p>
        <p>The Python strategy engine re-reads the active preset every 12 scan cycles to stay in sync.</p>
      </div>

      <BacktestPanel />
    </div>
  );
}
