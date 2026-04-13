import { useQuery } from "@tanstack/react-query";
import { Shield, Zap, BarChart3, CheckCircle } from "lucide-react";
import { cn } from "@/lib/utils";

interface MevStats {
  bundlesSubmitted: number;
  bundlesLanded: number;
  landedRate: number;
  mevSavedSol: number;
  jitoEnabled: boolean;
}

async function fetchMevStats(): Promise<MevStats> {
  const res = await fetch("/api/bot/mev-stats");
  if (!res.ok) throw new Error("Failed to fetch MEV stats");
  return res.json();
}

function StatItem({
  icon: Icon,
  label,
  value,
  sub,
  highlight,
}: {
  icon: React.ElementType;
  label: string;
  value: string;
  sub?: string;
  highlight?: boolean;
}) {
  return (
    <div className="flex items-start gap-3">
      <div className={cn("p-2 rounded-lg mt-0.5", highlight ? "bg-primary/10" : "bg-secondary")}>
        <Icon className={cn("w-4 h-4", highlight ? "text-primary" : "text-muted-foreground")} />
      </div>
      <div>
        <div className="text-xs text-muted-foreground">{label}</div>
        <div className={cn("text-sm font-semibold tabular-nums", highlight ? "text-primary" : "text-foreground")}>
          {value}
        </div>
        {sub && <div className="text-xs text-muted-foreground">{sub}</div>}
      </div>
    </div>
  );
}

export default function MevStatsPanel() {
  const { data, isLoading, isError } = useQuery<MevStats>({
    queryKey: ["mevStats"],
    queryFn: fetchMevStats,
    refetchInterval: 10_000,
    retry: 2,
  });

  return (
    <div className="bg-card border border-border rounded-xl p-5">
      <div className="flex items-center justify-between mb-4">
        <div className="flex items-center gap-2">
          <Shield className="w-4 h-4 text-primary" />
          <h2 className="text-sm font-semibold">Jito MEV Protection</h2>
        </div>
        {data && (
          <span
            className={cn(
              "text-xs px-2 py-0.5 rounded-full font-medium",
              data.jitoEnabled
                ? "bg-green-400/10 text-green-400"
                : "bg-muted text-muted-foreground"
            )}
          >
            {data.jitoEnabled ? "Jito Enabled" : "Direct RPC"}
          </span>
        )}
      </div>

      {isLoading && (
        <div className="space-y-3">
          {[1, 2, 3, 4].map((i) => (
            <div key={i} className="h-10 bg-secondary/50 rounded animate-pulse" />
          ))}
        </div>
      )}

      {isError && !isLoading && (
        <p className="text-xs text-muted-foreground text-center py-4">
          MEV stats unavailable — engine offline
        </p>
      )}

      {data && !isLoading && (
        <div className="grid grid-cols-2 gap-4">
          <StatItem
            icon={Zap}
            label="Bundles Submitted"
            value={data.bundlesSubmitted.toLocaleString()}
            highlight={data.bundlesSubmitted > 0}
          />
          <StatItem
            icon={CheckCircle}
            label="Bundles Landed"
            value={data.bundlesLanded.toLocaleString()}
            sub={`${data.landedRate.toFixed(1)}% success`}
            highlight={data.bundlesLanded > 0}
          />
          <StatItem
            icon={BarChart3}
            label="MEV Saved"
            value={`${data.mevSavedSol.toFixed(4)} SOL`}
            highlight={data.mevSavedSol > 0}
          />
          <StatItem
            icon={Shield}
            label="Sandwich Protection"
            value={data.jitoEnabled ? "Active" : "Inactive"}
            highlight={data.jitoEnabled}
          />
        </div>
      )}
    </div>
  );
}
