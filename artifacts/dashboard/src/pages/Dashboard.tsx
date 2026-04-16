import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { useListTrades, getGetPortfolioQueryOptions, getGetBotStatusQueryOptions, getGetMetricsQueryOptions } from "@workspace/api-client-react";
import { AreaChart, Area, XAxis, YAxis, Tooltip, ResponsiveContainer } from "recharts";
import {
  Activity, TrendingUp, TrendingDown, Wallet, Shield, Layers,
  Play, Square, Loader2, BarChart3,
} from "lucide-react";
import { cn, formatSol, formatPnl, formatPercent, shortenAddress } from "@/lib/utils";
import { useToast } from "@/hooks/use-toast";
import LiveTradesFeed from "@/components/LiveTradesFeed";
import MevStatsPanel from "@/components/MevStatsPanel";

function MetricCard({ label, value, sub, icon: Icon, trend }: {
  label: string;
  value: string;
  sub?: string;
  icon?: React.ElementType;
  trend?: "up" | "down" | "neutral";
}) {
  return (
    <div className="bg-card border border-border rounded-lg p-3 flex flex-col gap-1.5">
      <div className="flex items-center justify-between gap-1">
        <span className="text-[10px] sm:text-xs text-muted-foreground uppercase tracking-wider font-medium leading-tight">{label}</span>
        {Icon && <Icon className="w-3.5 h-3.5 text-muted-foreground flex-shrink-0" />}
      </div>
      <div className="flex items-end gap-1.5">
        <span className={cn(
          "text-base sm:text-xl font-bold tabular-nums leading-none",
          trend === "up" && "text-green-400",
          trend === "down" && "text-red-400",
        )}>{value}</span>
      </div>
      {sub && <span className="text-[10px] sm:text-xs text-muted-foreground leading-tight">{sub}</span>}
    </div>
  );
}

function StatusBadge({ connected, label }: { connected: boolean; label: string }) {
  return (
    <div className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-full bg-secondary/50 border border-border text-xs min-h-[32px]">
      <span className={cn("w-2 h-2 rounded-full flex-shrink-0", connected ? "bg-green-400 live-pulse" : "bg-red-400")} />
      <span className={cn("whitespace-nowrap", connected ? "text-green-400" : "text-red-400")}>{label}</span>
    </div>
  );
}

async function botControl(action: "start" | "stop") {
  const res = await fetch(`/api/bot/${action}`, { method: "POST" });
  if (!res.ok) throw new Error(`Failed to ${action} bot`);
  return res.json();
}

export default function DashboardPage() {
  const qc = useQueryClient();
  const { toast } = useToast();

  const { data: portfolio } = useQuery({
    ...getGetPortfolioQueryOptions(),
    refetchInterval: 5_000,
  });
  const { data: status } = useQuery({
    ...getGetBotStatusQueryOptions(),
    refetchInterval: 5_000,
  });
  const { data: metrics } = useQuery({
    ...getGetMetricsQueryOptions(),
    refetchInterval: 10_000,
  });

  const { data: tradesData } = useListTrades({ limit: 500 });
  const [chartWindow, setChartWindow] = useState<"24h" | "7d">("24h");

  // Advanced metrics from Python ML engine (Task #30)
  const { data: pyMetrics } = useQuery<Record<string, unknown>>({
    queryKey: ["/api/bot/metrics"],
    queryFn: async () => {
      const res = await fetch("/api/bot/metrics");
      if (!res.ok) return {};
      return res.json() as Promise<Record<string, unknown>>;
    },
    refetchInterval: 15_000,
  });

  const pnl = portfolio?.dailyPnlSol ?? 0;
  const totalPnl = portfolio?.totalPnlSol ?? 0;
  const isRunning = status?.running ?? false;
  const engineOffline = !isRunning && !(status?.pythonEngineRunning ?? false);

  const startBot = useMutation({
    mutationFn: () => botControl("start"),
    onSuccess: (data: { success: boolean; message: string }) => {
      if (data.success) {
        toast({ title: "Bot started", description: data.message });
        qc.invalidateQueries({ queryKey: ["/api/bot/status"] });
      } else {
        toast({ title: "Could not start bot", description: data.message, variant: "destructive" });
      }
    },
    onError: (err: Error) => {
      toast({ title: "Start failed", description: err.message, variant: "destructive" });
    },
  });
  const stopBot = useMutation({
    mutationFn: () => botControl("stop"),
    onSuccess: (data: { success: boolean; message: string }) => {
      if (data.success) {
        toast({ title: "Bot stopped", description: data.message });
        qc.invalidateQueries({ queryKey: ["/api/bot/status"] });
      } else {
        toast({ title: "Could not stop bot", description: data.message, variant: "destructive" });
      }
    },
    onError: (err: Error) => {
      toast({ title: "Stop failed", description: err.message, variant: "destructive" });
    },
  });

  const ctrlPending = startBot.isPending || stopBot.isPending;

  const pnlChartData = (() => {
    const trades = tradesData ?? [];
    const now = Date.now();

    if (chartWindow === "24h") {
      const msPerHour = 60 * 60 * 1000;
      const cutoff = now - 24 * msPerHour;
      const epochHourOf = (ms: number) => Math.floor(ms / msPerHour);
      const currentEpochHour = epochHourOf(now);
      const hourMap: Record<number, number> = {};
      for (const t of trades) {
        if (!t.createdAt) continue;
        const ts = new Date(t.createdAt).getTime();
        if (ts < cutoff) continue;
        const bucket = epochHourOf(ts);
        hourMap[bucket] = (hourMap[bucket] ?? 0) + (t.pnlSol ?? 0);
      }
      let cumulative = 0;
      return Array.from({ length: 24 }, (_, i) => {
        const bucket = currentEpochHour - 23 + i;
        const label = `${String(new Date(bucket * msPerHour).getHours()).padStart(2, "0")}:00`;
        cumulative += hourMap[bucket] ?? 0;
        return { time: label, pnl: cumulative };
      });
    } else {
      const cutoff = now - 7 * 24 * 60 * 60 * 1000;
      const dayMap: Record<string, number> = {};
      for (const t of trades) {
        if (!t.createdAt) continue;
        const d = new Date(t.createdAt);
        if (d.getTime() < cutoff) continue;
        const day = d.toLocaleDateString("en-US", { month: "short", day: "numeric" });
        dayMap[day] = (dayMap[day] ?? 0) + (t.pnlSol ?? 0);
      }
      const days: string[] = [];
      for (let i = 6; i >= 0; i--) {
        const d = new Date(now - i * 24 * 60 * 60 * 1000);
        days.push(d.toLocaleDateString("en-US", { month: "short", day: "numeric" }));
      }
      let cumulative = 0;
      return days.map((day) => {
        cumulative += dayMap[day] ?? 0;
        return { time: day, pnl: cumulative };
      });
    }
  })();

  const activeStrategies = status?.activeStrategies ?? [];

  return (
    <div className="space-y-4 sm:space-y-6">
      {/* Status bar with bot controls */}
      <div className="space-y-2">
        {/* Status badges row */}
        <div className="flex flex-wrap items-center gap-2">
          {engineOffline ? (
            <div className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-full bg-secondary/50 border border-border text-xs min-h-[32px]">
              <span className="w-2 h-2 rounded-full flex-shrink-0 bg-red-400" />
              <span className="text-red-400 whitespace-nowrap">Engine offline</span>
            </div>
          ) : (
            <StatusBadge connected={isRunning} label="Bot Running" />
          )}
          <StatusBadge connected={status?.rustEngineConnected ?? false} label="Rust Engine" />
          <StatusBadge connected={status?.pythonEngineRunning ?? false} label="Python ML" />
          {status?.walletAddress && (
            <div className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-full bg-secondary/50 border border-border text-xs text-muted-foreground min-h-[32px]">
              <Wallet className="w-3.5 h-3.5 flex-shrink-0" />
              <span className="font-mono">{shortenAddress(status.walletAddress)}</span>
            </div>
          )}
          {status?.environment && (
            <div className="flex items-center gap-1.5 px-2.5 py-1.5 rounded-full bg-amber-500/10 border border-amber-500/30 text-xs text-amber-400 min-h-[32px]">
              <span className="uppercase tracking-wider">{status.environment}</span>
            </div>
          )}
          {activeStrategies.length > 0 && (
            <div className="flex items-center gap-1 text-xs">
              <span className="text-muted-foreground">Active:</span>
              {activeStrategies.map((s) => (
                <span key={s} className="px-1.5 py-0.5 rounded bg-primary/10 text-primary capitalize">{s}</span>
              ))}
            </div>
          )}
        </div>

        {/* Start / Stop — full-width on mobile, auto on desktop */}
        <button
          onClick={() => isRunning ? stopBot.mutate() : startBot.mutate()}
          disabled={ctrlPending}
          className={cn(
            "flex items-center justify-center gap-2 w-full sm:w-auto sm:px-5 px-4 py-3 sm:py-2 rounded-lg text-sm font-semibold transition-opacity min-h-[44px]",
            isRunning
              ? "bg-red-500/10 text-red-400 border border-red-500/30 hover:bg-red-500/20"
              : "bg-green-500/10 text-green-400 border border-green-500/30 hover:bg-green-500/20",
            ctrlPending && "opacity-50 cursor-not-allowed"
          )}
        >
          {ctrlPending
            ? <Loader2 className="w-4 h-4 animate-spin" />
            : isRunning
              ? <Square className="w-4 h-4" />
              : <Play className="w-4 h-4" />}
          {isRunning ? "Stop Bot" : "Start Bot"}
        </button>
      </div>

      {/* Portfolio metrics */}
      <div className="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-6 gap-2 sm:gap-3">
        <MetricCard
          label="Total Value"
          value={formatSol(portfolio?.totalValueSol ?? 0)}
          icon={Layers}
          trend="neutral"
        />
        <MetricCard
          label="Cash Balance"
          value={formatSol(portfolio?.cashBalanceSol ?? 0)}
          icon={Wallet}
          trend="neutral"
        />
        <MetricCard
          label="Positions"
          value={formatSol(portfolio?.positionsValueSol ?? 0)}
          icon={Activity}
          trend="neutral"
        />
        <MetricCard
          label="Daily PnL"
          value={formatPnl(pnl)}
          sub={pnl >= 0 ? "Profitable today" : "Today's loss"}
          icon={pnl >= 0 ? TrendingUp : TrendingDown}
          trend={pnl >= 0 ? "up" : "down"}
        />
        <MetricCard
          label="Total PnL"
          value={formatPnl(totalPnl)}
          icon={Activity}
          trend={totalPnl >= 0 ? "up" : "down"}
        />
        <MetricCard
          label="Win Rate"
          value={formatPercent(portfolio?.winRate ?? 0)}
          sub={`${portfolio?.openPositionsCount ?? 0} open`}
          icon={Shield}
          trend={(portfolio?.winRate ?? 0) > 50 ? "up" : "down"}
        />
      </div>

      {/* PnL chart + Engine metrics */}
      <div className="grid grid-cols-1 lg:grid-cols-3 gap-4">
        <div className="lg:col-span-2 bg-card border border-border rounded-lg p-4">
          <div className="flex items-center justify-between mb-4">
            <h2 className="text-sm font-semibold">Cumulative PnL</h2>
            <div className="flex items-center gap-2">
              <div className="flex rounded-md overflow-hidden border border-border text-xs">
                {(["24h", "7d"] as const).map((w) => (
                  <button
                    key={w}
                    onClick={() => setChartWindow(w)}
                    className={cn(
                      "px-2.5 py-1 transition-colors",
                      chartWindow === w
                        ? "bg-primary/20 text-primary font-medium"
                        : "text-muted-foreground hover:text-foreground"
                    )}
                  >
                    {w}
                  </button>
                ))}
              </div>
              <span className={cn("text-xs font-medium tabular-nums",
                pnl >= 0 ? "text-green-400" : "text-red-400"
              )}>
                {formatPnl(pnl)} today
              </span>
            </div>
          </div>
          <ResponsiveContainer width="100%" height={160}>
            <AreaChart data={pnlChartData}>
              <defs>
                <linearGradient id="pnlGrad" x1="0" y1="0" x2="0" y2="1">
                  <stop offset="5%" stopColor="#22c55e" stopOpacity={0.2} />
                  <stop offset="95%" stopColor="#22c55e" stopOpacity={0} />
                </linearGradient>
              </defs>
              <XAxis dataKey="time" tick={{ fontSize: 10, fill: "#6b7280" }} tickLine={false} axisLine={false} interval="preserveStartEnd" />
              <YAxis tick={{ fontSize: 10, fill: "#6b7280" }} tickLine={false} axisLine={false} tickFormatter={(v: number) => v.toFixed(2)} width={40} />
              <Tooltip
                contentStyle={{ background: "hsl(222 47% 11%)", border: "1px solid hsl(217 33% 17%)", borderRadius: 6, fontSize: 11 }}
                itemStyle={{ color: "#22c55e" }}
                formatter={(v: number) => [v.toFixed(4) + " SOL", "PnL"]}
              />
              <Area type="monotone" dataKey="pnl" stroke="#22c55e" strokeWidth={1.5} fill="url(#pnlGrad)" dot={false} />
            </AreaChart>
          </ResponsiveContainer>
        </div>

        <div className="bg-card border border-border rounded-lg p-4 space-y-3">
          <h2 className="text-sm font-semibold mb-2">Engine Metrics</h2>
          {[
            { label: "Orders Executed", value: metrics?.ordersExecuted ?? 0 },
            { label: "Orders Failed", value: metrics?.ordersFailed ?? 0, color: "text-red-400" },
            { label: "Jito Bundles", value: metrics?.jitoLanded ?? 0, color: "text-blue-400" },
            { label: "Sandwiches Blocked", value: metrics?.sandwichAttacks ?? 0, color: "text-amber-400" },
            { label: "Tokens Sniped", value: metrics?.tokensSniped ?? 0, color: "text-purple-400" },
            { label: "Avg Exec (ms)", value: (metrics?.avgExecutionMs ?? 0).toFixed(0) },
          ].map((row) => (
            <div key={row.label} className="flex items-center justify-between text-xs">
              <span className="text-muted-foreground">{row.label}</span>
              <span className={cn("tabular-nums font-medium", row.color ?? "text-foreground")}>
                {row.value}
              </span>
            </div>
          ))}
        </div>
      </div>

      {/* Advanced strategy metrics (Task #30) */}
      {pyMetrics && (
        <div className="grid grid-cols-2 sm:grid-cols-4 gap-2 sm:gap-3">
          <div className="bg-card border border-border rounded-lg p-3">
            <div className="flex items-center justify-between mb-1.5">
              <span className="text-[10px] sm:text-xs text-muted-foreground uppercase tracking-wider font-medium">Sharpe Ratio</span>
              <BarChart3 className="w-3.5 h-3.5 text-muted-foreground" />
            </div>
            <span className={cn(
              "text-base sm:text-xl font-bold tabular-nums",
              Number(pyMetrics.sharpe_ratio ?? 0) >= 1 ? "text-green-400" : "text-muted-foreground"
            )}>
              {Number(pyMetrics.sharpe_ratio ?? 0).toFixed(2)}
            </span>
            <div className="text-[10px] text-muted-foreground mt-0.5">annualised</div>
          </div>
          <div className="bg-card border border-border rounded-lg p-3">
            <div className="flex items-center justify-between mb-1.5">
              <span className="text-[10px] sm:text-xs text-muted-foreground uppercase tracking-wider font-medium">Max Drawdown</span>
              <TrendingDown className="w-3.5 h-3.5 text-red-400" />
            </div>
            <span className="text-base sm:text-xl font-bold tabular-nums text-red-400">
              {Number(pyMetrics.max_drawdown_sol ?? 0).toFixed(4)} SOL
            </span>
            <div className="text-[10px] text-muted-foreground mt-0.5">peak-to-trough</div>
          </div>
          <div className="bg-card border border-border rounded-lg p-3">
            <div className="flex items-center justify-between mb-1.5">
              <span className="text-[10px] sm:text-xs text-muted-foreground uppercase tracking-wider font-medium">Volatility</span>
              <Activity className="w-3.5 h-3.5 text-amber-400" />
            </div>
            <span className="text-base sm:text-xl font-bold tabular-nums text-amber-400">
              {Number(pyMetrics.volatility_sol ?? 0).toFixed(4)} SOL
            </span>
            <div className="text-[10px] text-muted-foreground mt-0.5">per trade σ</div>
          </div>
          <div className="bg-card border border-border rounded-lg p-3">
            <div className="flex items-center justify-between mb-1.5">
              <span className="text-[10px] sm:text-xs text-muted-foreground uppercase tracking-wider font-medium">Circuit Breaker</span>
              <Shield className="w-3.5 h-3.5 text-muted-foreground" />
            </div>
            <span className={cn(
              "text-base sm:text-xl font-bold tabular-nums",
              String(pyMetrics.circuit_breaker_state ?? "CLOSED") === "CLOSED" ? "text-green-400" :
              String(pyMetrics.circuit_breaker_state ?? "CLOSED") === "HALF_OPEN" ? "text-amber-400" : "text-red-400"
            )}>
              {String(pyMetrics.circuit_breaker_state ?? "CLOSED")}
            </span>
            <div className="text-[10px] text-muted-foreground mt-0.5">order submission</div>
          </div>
        </div>
      )}

      {/* MEV stats + Live trade feed */}
      <div className="grid grid-cols-1 lg:grid-cols-3 gap-4">
        <div className="lg:col-span-1">
          <MevStatsPanel />
        </div>
        <div className="lg:col-span-2">
          <LiveTradesFeed />
        </div>
      </div>
    </div>
  );
}
