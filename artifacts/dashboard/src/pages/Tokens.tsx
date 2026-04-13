import { useListTokens } from "@workspace/api-client-react";
import { cn, formatSol } from "@/lib/utils";
import { CircleDot } from "lucide-react";

function BondingCurveBar({ progress }: { progress: number }) {
  return (
    <div className="flex items-center gap-2">
      <div className="flex-1 h-1.5 bg-secondary rounded-full overflow-hidden">
        <div
          className={cn("h-full rounded-full transition-all",
            progress < 30 ? "bg-green-400" : progress < 70 ? "bg-amber-400" : "bg-red-400"
          )}
          style={{ width: `${Math.min(progress, 100)}%` }}
        />
      </div>
      <span className="text-xs tabular-nums text-muted-foreground w-10 text-right">{progress.toFixed(0)}%</span>
    </div>
  );
}

function ActionBadge({ action }: { action?: string }) {
  if (!action) return <span className="text-muted-foreground">—</span>;
  const colorClass = action === "SNIPED"
    ? "bg-green-500/10 text-green-400"
    : action === "WATCHED"
    ? "bg-amber-500/10 text-amber-400"
    : "bg-secondary text-muted-foreground";
  return (
    <span className={cn("px-1.5 py-0.5 rounded text-xs font-medium", colorClass)}>
      {action}
    </span>
  );
}

function formatAge(detectedAt?: string): string {
  if (!detectedAt) return "—";
  const ms = Date.now() - new Date(detectedAt).getTime();
  if (ms < 60_000) return `${Math.floor(ms / 1000)}s ago`;
  if (ms < 3_600_000) return `${Math.floor(ms / 60_000)}m ago`;
  return `${Math.floor(ms / 3_600_000)}h ago`;
}

export default function TokensPage() {
  const { data: tokens, isLoading } = useListTokens();

  const tokenList = tokens ?? [];

  if (isLoading) {
    return <div className="text-muted-foreground text-sm">Loading tokens...</div>;
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center gap-3">
        <h1 className="text-lg font-bold">Token Monitor</h1>
        <div className="flex items-center gap-1.5 text-xs text-muted-foreground">
          <span className="w-2 h-2 rounded-full bg-green-400 live-pulse" />
          <span>{tokenList.length} tracked</span>
        </div>
      </div>

      {tokenList.length === 0 ? (
        <div className="bg-card border border-border rounded-lg p-8 text-center">
          <CircleDot className="w-8 h-8 text-muted-foreground mx-auto mb-3" />
          <p className="text-muted-foreground text-sm">No tokens currently tracked.</p>
          <p className="text-muted-foreground text-xs mt-1">Tokens appear here as the bot discovers new launches.</p>
        </div>
      ) : (
        <div className="bg-card border border-border rounded-lg overflow-hidden overflow-x-auto">
          <table className="w-full text-xs min-w-max">
            <thead>
              <tr className="text-muted-foreground border-b border-border bg-secondary/20">
                <th className="text-left py-3 px-4 font-medium">Token</th>
                <th className="text-right py-3 px-4 font-medium">Detected</th>
                <th className="text-right py-3 px-4 font-medium">Price (SOL)</th>
                <th className="text-right py-3 px-4 font-medium">Market Cap</th>
                <th className="text-right py-3 px-4 font-medium">Liquidity</th>
                <th className="text-right py-3 px-4 font-medium">24h Volume</th>
                <th className="text-right py-3 px-4 font-medium">Holders</th>
                <th className="text-right py-3 px-4 font-medium">ML Score</th>
                <th className="text-center py-3 px-4 font-medium">Action Taken</th>
                <th className="py-3 px-4 font-medium">BC Progress</th>
              </tr>
            </thead>
            <tbody>
              {tokenList.map((token) => (
                <tr key={token.mint} className="border-b border-border/50 hover:bg-secondary/20 transition-colors">
                  <td className="py-3 px-4">
                    <div className="flex flex-col">
                      <span className="font-semibold text-foreground">{token.symbol ?? "—"}</span>
                      <span className="text-muted-foreground font-mono text-xs">
                        {token.mint?.slice(0, 8)}...
                      </span>
                    </div>
                  </td>
                  <td className="py-3 px-4 text-right tabular-nums text-muted-foreground whitespace-nowrap">
                    {formatAge(token.detectedAt)}
                  </td>
                  <td className="py-3 px-4 text-right tabular-nums">
                    {token.price < 0.000001
                      ? token.price.toExponential(2)
                      : token.price.toFixed(8)}
                  </td>
                  <td className="py-3 px-4 text-right tabular-nums">{formatSol(token.marketCapSol, 1)}</td>
                  <td className="py-3 px-4 text-right tabular-nums">{formatSol(token.liquiditySol, 2)}</td>
                  <td className="py-3 px-4 text-right tabular-nums">{formatSol(token.volume24hSol ?? 0, 1)}</td>
                  <td className="py-3 px-4 text-right tabular-nums">{token.holderCount ?? "—"}</td>
                  <td className="py-3 px-4 text-right tabular-nums">
                    {token.mlScore != null ? (
                      <span className={cn(
                        "px-1.5 py-0.5 rounded text-xs font-medium tabular-nums",
                        token.mlScore >= 0.7 ? "bg-green-400/10 text-green-400"
                          : token.mlScore >= 0.4 ? "bg-amber-400/10 text-amber-400"
                          : "bg-red-400/10 text-red-400"
                      )}>
                        {(token.mlScore * 100).toFixed(0)}%
                      </span>
                    ) : "—"}
                  </td>
                  <td className="py-3 px-4 text-center">
                    <ActionBadge action={token.actionTaken} />
                  </td>
                  <td className="py-3 px-4 min-w-32">
                    <BondingCurveBar progress={token.bondingCurveProgress} />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}
