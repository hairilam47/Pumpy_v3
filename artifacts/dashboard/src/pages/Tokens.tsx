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

export default function TokensPage() {
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const { data: tokens, isLoading } = useListTokens() as { data: any[] | undefined; isLoading: boolean };

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
        <div className="bg-card border border-border rounded-lg overflow-hidden">
          <table className="w-full text-xs">
            <thead>
              <tr className="text-muted-foreground border-b border-border bg-secondary/20">
                <th className="text-left py-3 px-4 font-medium">Token</th>
                <th className="text-right py-3 px-4 font-medium">Price (SOL)</th>
                <th className="text-right py-3 px-4 font-medium">Market Cap</th>
                <th className="text-right py-3 px-4 font-medium">Liquidity</th>
                <th className="text-right py-3 px-4 font-medium">24h Volume</th>
                <th className="text-right py-3 px-4 font-medium">Holders</th>
                <th className="py-3 px-4 font-medium">BC Progress</th>
              </tr>
            </thead>
            <tbody>
              {tokenList.map((token) => (
                <tr key={token.mint} className="border-b border-border/50 hover:bg-secondary/20 transition-colors">
                  <td className="py-3 px-4">
                    <div className="flex flex-col">
                      <span className="font-semibold text-foreground">{token.symbol || "—"}</span>
                      <span className="text-muted-foreground font-mono text-xs">
                        {token.mint?.slice(0, 8)}...
                      </span>
                    </div>
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
