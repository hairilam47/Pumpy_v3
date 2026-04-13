import { useGetPortfolio, useGetBotStatus, useGetMetrics, useListTrades } from "@workspace/api-client-react";
import { useQuery } from "@tanstack/react-query";
import { AreaChart, Area, XAxis, YAxis, Tooltip, ResponsiveContainer } from "recharts";
import { Activity, TrendingUp, TrendingDown, Wallet, Shield, Zap, ArrowUpRight, ArrowDownRight } from "lucide-react";
import { cn, formatSol, formatPnl, formatPercent, shortenAddress, formatAge } from "@/lib/utils";

function MetricCard({ label, value, sub, icon: Icon, trend }: {
  label: string;
  value: string;
  sub?: string;
  icon?: React.ElementType;
  trend?: "up" | "down" | "neutral";
}) {
  return (
    <div className="bg-card border border-border rounded-lg p-4 flex flex-col gap-2">
      <div className="flex items-center justify-between">
        <span className="text-xs text-muted-foreground uppercase tracking-wider font-medium">{label}</span>
        {Icon && <Icon className="w-4 h-4 text-muted-foreground" />}
      </div>
      <div className="flex items-end gap-2">
        <span className={cn("text-xl font-bold tabular-nums",
          trend === "up" && "text-green-400",
          trend === "down" && "text-red-400",
        )}>{value}</span>
      </div>
      {sub && <span className="text-xs text-muted-foreground">{sub}</span>}
    </div>
  );
}

function StatusBadge({ connected, label }: { connected: boolean; label: string }) {
  return (
    <div className="flex items-center gap-2 px-3 py-1.5 rounded-full bg-secondary/50 border border-border text-xs">
      <span className={cn("w-2 h-2 rounded-full", connected ? "bg-green-400 live-pulse" : "bg-red-400")} />
      <span className={cn(connected ? "text-green-400" : "text-red-400")}>{label}</span>
    </div>
  );
}

const MOCK_PNL_DATA = Array.from({ length: 24 }, (_, i) => ({
  time: `${String(i).padStart(2, "0")}:00`,
  pnl: (Math.sin(i / 4) * 0.3 + Math.random() * 0.1 - 0.03) * (1 + i * 0.02),
}));

export default function DashboardPage() {
  const { data: portfolio } = useGetPortfolio();
  const { data: status } = useGetBotStatus();
  const { data: metrics } = useGetMetrics();
  const { data: trades } = useListTrades({ limit: 10 });

  const pnl = portfolio?.dailyPnlSol ?? 0;
  const totalPnl = portfolio?.totalPnlSol ?? 0;

  return (
    <div className="space-y-6">
      {/* Status bar */}
      <div className="flex flex-wrap items-center gap-2">
        <StatusBadge connected={status?.running ?? false} label="Bot Running" />
        <StatusBadge connected={status?.rustEngineConnected ?? false} label="Rust Engine" />
        <StatusBadge connected={status?.pythonEngineRunning ?? false} label="Python ML" />
        {status?.walletAddress && (
          <div className="flex items-center gap-2 px-3 py-1.5 rounded-full bg-secondary/50 border border-border text-xs text-muted-foreground">
            <Wallet className="w-3.5 h-3.5" />
            <span className="font-mono">{shortenAddress(status.walletAddress)}</span>
          </div>
        )}
        {status?.environment && (
          <div className="ml-auto flex items-center gap-1.5 px-3 py-1.5 rounded-full bg-amber-500/10 border border-amber-500/30 text-xs text-amber-400">
            <span className="uppercase tracking-wider">{status.environment}</span>
          </div>
        )}
      </div>

      {/* Portfolio metrics */}
      <div className="grid grid-cols-2 md:grid-cols-4 gap-4">
        <MetricCard
          label="Wallet Balance"
          value={formatSol(portfolio?.cashBalanceSol ?? 0)}
          icon={Wallet}
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
          sub={`${portfolio?.openPositionsCount ?? 0} open positions`}
          icon={Shield}
          trend={(portfolio?.winRate ?? 0) > 50 ? "up" : "down"}
        />
      </div>

      {/* PnL chart + Metrics */}
      <div className="grid grid-cols-1 lg:grid-cols-3 gap-4">
        {/* PnL Chart */}
        <div className="lg:col-span-2 bg-card border border-border rounded-lg p-4">
          <div className="flex items-center justify-between mb-4">
            <h2 className="text-sm font-semibold">24h PnL</h2>
            <span className={cn("text-xs font-medium tabular-nums",
              pnl >= 0 ? "text-green-400" : "text-red-400"
            )}>
              {formatPnl(pnl)} today
            </span>
          </div>
          <ResponsiveContainer width="100%" height={160}>
            <AreaChart data={MOCK_PNL_DATA}>
              <defs>
                <linearGradient id="pnlGrad" x1="0" y1="0" x2="0" y2="1">
                  <stop offset="5%" stopColor="#22c55e" stopOpacity={0.2} />
                  <stop offset="95%" stopColor="#22c55e" stopOpacity={0} />
                </linearGradient>
              </defs>
              <XAxis dataKey="time" tick={{ fontSize: 10, fill: "#6b7280" }} tickLine={false} axisLine={false} />
              <YAxis tick={{ fontSize: 10, fill: "#6b7280" }} tickLine={false} axisLine={false} tickFormatter={(v: number) => v.toFixed(2)} />
              <Tooltip
                contentStyle={{ background: "hsl(222 47% 11%)", border: "1px solid hsl(217 33% 17%)", borderRadius: 6, fontSize: 11 }}
                itemStyle={{ color: "#22c55e" }}
                formatter={(v: number) => [v.toFixed(4) + " SOL", "PnL"]}
              />
              <Area type="monotone" dataKey="pnl" stroke="#22c55e" strokeWidth={1.5} fill="url(#pnlGrad)" dot={false} />
            </AreaChart>
          </ResponsiveContainer>
        </div>

        {/* Bot metrics */}
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

      {/* Recent trades */}
      <div className="bg-card border border-border rounded-lg p-4">
        <div className="flex items-center gap-2 mb-4">
          <h2 className="text-sm font-semibold">Recent Trades</h2>
          <span className="w-2 h-2 rounded-full bg-green-400 live-pulse" />
          <span className="text-xs text-muted-foreground">Live</span>
        </div>
        <div className="overflow-x-auto">
          <table className="w-full text-xs">
            <thead>
              <tr className="text-muted-foreground border-b border-border">
                <th className="text-left py-2 pr-4 font-medium">Token</th>
                <th className="text-left py-2 pr-4 font-medium">Side</th>
                <th className="text-right py-2 pr-4 font-medium">Amount</th>
                <th className="text-left py-2 pr-4 font-medium">Strategy</th>
                <th className="text-left py-2 pr-4 font-medium">Status</th>
                <th className="text-right py-2 font-medium">Time</th>
              </tr>
            </thead>
            <tbody>
              {trades && trades.length > 0 ? (
                // eslint-disable-next-line @typescript-eslint/no-explicit-any
                (trades as any[]).map((trade) => (
                  <tr key={trade.id} className="border-b border-border/50 hover:bg-secondary/30 transition-colors">
                    <td className="py-2 pr-4">
                      <div className="flex flex-col">
                        <span className="font-medium text-foreground">{trade.tokenSymbol || "—"}</span>
                        <span className="text-muted-foreground font-mono">{shortenAddress(trade.mint)}</span>
                      </div>
                    </td>
                    <td className="py-2 pr-4">
                      <span className={cn("px-2 py-0.5 rounded text-xs font-semibold",
                        trade.side === "BUY" ? "bg-green-400/10 text-green-400" : "bg-red-400/10 text-red-400"
                      )}>
                        {trade.side === "BUY" ? <ArrowUpRight className="inline w-3 h-3 mr-0.5" /> : <ArrowDownRight className="inline w-3 h-3 mr-0.5" />}
                        {trade.side}
                      </span>
                    </td>
                    <td className="py-2 pr-4 text-right tabular-nums">{formatSol(trade.amountSol)}</td>
                    <td className="py-2 pr-4">
                      <span className="px-2 py-0.5 rounded bg-secondary text-muted-foreground text-xs">
                        {trade.strategy}
                      </span>
                    </td>
                    <td className="py-2 pr-4">
                      <StatusChip status={trade.status} />
                    </td>
                    <td className="py-2 text-right text-muted-foreground">{formatAge(trade.createdAt)}</td>
                  </tr>
                ))
              ) : (
                <tr>
                  <td colSpan={6} className="py-8 text-center text-muted-foreground">
                    No trades yet — bot will populate data as it runs
                  </td>
                </tr>
              )}
            </tbody>
          </table>
        </div>
      </div>
    </div>
  );
}

function StatusChip({ status }: { status: string }) {
  const cfg: Record<string, { bg: string; text: string }> = {
    Executed: { bg: "bg-green-400/10", text: "text-green-400" },
    Pending: { bg: "bg-amber-400/10", text: "text-amber-400" },
    Executing: { bg: "bg-blue-400/10", text: "text-blue-400" },
    Failed: { bg: "bg-red-400/10", text: "text-red-400" },
    Cancelled: { bg: "bg-muted", text: "text-muted-foreground" },
  };
  const style = cfg[status] || { bg: "bg-muted", text: "text-muted-foreground" };
  return (
    <span className={cn("px-2 py-0.5 rounded text-xs font-medium", style.bg, style.text)}>
      {status}
    </span>
  );
}
