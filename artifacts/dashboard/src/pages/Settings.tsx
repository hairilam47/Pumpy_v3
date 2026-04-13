import { useQuery } from "@tanstack/react-query";
import { CheckCircle, XCircle, AlertCircle, Wifi, WifiOff, Key, Server, Settings2 } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";

interface EnvVar {
  key: string;
  required: boolean;
  description: string;
  setIn: string;
  set: boolean;
  masked: string;
}

interface SettingsStatus {
  wallet: {
    pubkey: string | null;
    source: string | null;
    configured: boolean;
  };
  rpc: {
    url: string | null;
    configured: boolean;
    latencyMs: number | null;
    online: boolean;
  };
  envVars: EnvVar[];
}

function useSettingsStatus() {
  return useQuery<SettingsStatus>({
    queryKey: ["settingsStatus"],
    queryFn: async () => {
      const res = await fetch("/api/settings/status");
      if (!res.ok) throw new Error("Failed to fetch settings status");
      return res.json() as Promise<SettingsStatus>;
    },
    refetchInterval: 30_000,
    staleTime: 20_000,
  });
}

function StatusIcon({ ok }: { ok: boolean }) {
  return ok ? (
    <CheckCircle className="w-4 h-4 text-green-500 flex-shrink-0" />
  ) : (
    <XCircle className="w-4 h-4 text-red-500 flex-shrink-0" />
  );
}

function LatencyBadge({ ms }: { ms: number | null }) {
  if (ms === null) return <Badge variant="destructive" className="text-xs">Offline</Badge>;
  const color =
    ms < 200
      ? "bg-green-500/15 text-green-400"
      : ms < 600
      ? "bg-yellow-500/15 text-yellow-400"
      : "bg-red-500/15 text-red-400";
  return (
    <span className={`text-xs font-mono px-2 py-0.5 rounded ${color}`}>
      {ms} ms
    </span>
  );
}

export default function SettingsPage() {
  const { data, isLoading, error, dataUpdatedAt } = useSettingsStatus();

  const lastUpdated = dataUpdatedAt ? new Date(dataUpdatedAt).toLocaleTimeString() : null;

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold text-foreground flex items-center gap-2">
            <Settings2 className="w-6 h-6 text-primary" />
            Settings &amp; Configuration
          </h1>
          <p className="text-sm text-muted-foreground mt-1">
            Read-only view of environment configuration — no secrets are ever exposed.
          </p>
        </div>
        {lastUpdated && (
          <p className="text-xs text-muted-foreground">
            Updated {lastUpdated} &middot; refreshes every 30s
          </p>
        )}
      </div>

      {isLoading && (
        <div className="text-sm text-muted-foreground animate-pulse">
          Loading configuration status…
        </div>
      )}
      {error && (
        <div className="flex items-center gap-2 text-sm text-destructive">
          <AlertCircle className="w-4 h-4" />
          Failed to load settings — API server may be offline.
        </div>
      )}

      {data && (
        <>
          {/* Wallet */}
          <Card>
            <CardHeader className="pb-3">
              <CardTitle className="text-base flex items-center gap-2">
                <Key className="w-4 h-4 text-primary" />
                Wallet
              </CardTitle>
            </CardHeader>
            <CardContent className="space-y-3">
              <div className="flex items-center gap-3">
                <StatusIcon ok={data.wallet.configured} />
                <div className="flex-1">
                  <div className="text-sm font-medium text-foreground">
                    {data.wallet.configured ? "Wallet configured" : "No wallet configured"}
                  </div>
                  {data.wallet.source && (
                    <div className="text-xs text-muted-foreground mt-0.5">
                      Source:{" "}
                      <span className="font-mono">{data.wallet.source}</span>
                    </div>
                  )}
                </div>
                {data.wallet.configured && (
                  <Badge variant="secondary" className="text-xs">
                    {data.wallet.source === "WALLET_PRIVATE_KEY" ? "Env Secret" : "Key File"}
                  </Badge>
                )}
              </div>

              {data.wallet.pubkey ? (
                <div className="rounded-lg bg-muted/50 px-3 py-2.5">
                  <div className="text-xs text-muted-foreground mb-1">Public Key</div>
                  <div className="font-mono text-sm text-foreground break-all">
                    {data.wallet.pubkey}
                  </div>
                </div>
              ) : data.wallet.configured ? (
                <div className="rounded-lg bg-muted/30 px-3 py-2 text-xs text-muted-foreground">
                  Public key is available when the Rust engine starts, or when{" "}
                  <span className="font-mono">WALLET_PRIVATE_KEY</span> is set as a
                  JSON byte array.
                </div>
              ) : (
                <div className="rounded-lg bg-destructive/10 px-3 py-2 text-xs text-destructive">
                  Set <span className="font-mono">WALLET_PRIVATE_KEY</span> (base58 or JSON
                  array) in the{" "}
                  <span className="font-semibold">Replit Secrets panel</span>, or set{" "}
                  <span className="font-mono">KEYPAIR_PATH</span> to a keypair file path.
                </div>
              )}
            </CardContent>
          </Card>

          {/* RPC Connection */}
          <Card>
            <CardHeader className="pb-3">
              <CardTitle className="text-base flex items-center gap-2">
                {data.rpc.online ? (
                  <Wifi className="w-4 h-4 text-primary" />
                ) : (
                  <WifiOff className="w-4 h-4 text-muted-foreground" />
                )}
                RPC Connection
              </CardTitle>
            </CardHeader>
            <CardContent className="space-y-3">
              <div className="flex items-center gap-3">
                <StatusIcon ok={data.rpc.configured} />
                <div className="flex-1">
                  <div className="text-sm font-medium text-foreground">
                    {data.rpc.configured
                      ? "RPC endpoint configured"
                      : "No RPC endpoint configured"}
                  </div>
                  {data.rpc.url && (
                    <div className="text-xs text-muted-foreground font-mono mt-0.5 truncate max-w-xs">
                      {data.rpc.url}
                    </div>
                  )}
                </div>
                <LatencyBadge ms={data.rpc.latencyMs} />
              </div>

              {!data.rpc.configured && (
                <div className="rounded-lg bg-destructive/10 px-3 py-2 text-xs text-destructive">
                  Set <span className="font-mono">SOLANA_RPC_URL</span> in the{" "}
                  <span className="font-semibold">Replit Secrets panel</span>.{" "}
                  For multi-endpoint failover use{" "}
                  <span className="font-mono">SOLANA_RPC_URLS</span> (comma-separated).
                </div>
              )}

              {data.rpc.configured && (
                <div className="grid grid-cols-3 gap-3">
                  <div className="rounded-lg bg-muted/40 px-3 py-2 text-center">
                    <div className="text-xs text-muted-foreground">Status</div>
                    <div
                      className={`text-sm font-semibold mt-0.5 ${
                        data.rpc.online ? "text-green-400" : "text-red-400"
                      }`}
                    >
                      {data.rpc.online ? "Online" : "Offline"}
                    </div>
                  </div>
                  <div className="rounded-lg bg-muted/40 px-3 py-2 text-center">
                    <div className="text-xs text-muted-foreground">Latency</div>
                    <div className="text-sm font-semibold mt-0.5 font-mono">
                      {data.rpc.latencyMs !== null ? `${data.rpc.latencyMs}ms` : "—"}
                    </div>
                  </div>
                  <div className="rounded-lg bg-muted/40 px-3 py-2 text-center">
                    <div className="text-xs text-muted-foreground">Protocol</div>
                    <div className="text-sm font-semibold mt-0.5">JSON-RPC</div>
                  </div>
                </div>
              )}
            </CardContent>
          </Card>

          {/* Environment Variables Table */}
          <Card>
            <CardHeader className="pb-3">
              <CardTitle className="text-base flex items-center gap-2">
                <Server className="w-4 h-4 text-primary" />
                Environment Variables
              </CardTitle>
            </CardHeader>
            <CardContent>
              <div className="rounded-lg border border-border overflow-hidden">
                <table className="w-full text-sm">
                  <thead>
                    <tr className="bg-muted/50">
                      <th className="text-left px-3 py-2.5 text-xs font-semibold text-muted-foreground uppercase tracking-wide w-8" />
                      <th className="text-left px-3 py-2.5 text-xs font-semibold text-muted-foreground uppercase tracking-wide">
                        Variable
                      </th>
                      <th className="text-left px-3 py-2.5 text-xs font-semibold text-muted-foreground uppercase tracking-wide hidden md:table-cell">
                        Required
                      </th>
                      <th className="text-left px-3 py-2.5 text-xs font-semibold text-muted-foreground uppercase tracking-wide hidden lg:table-cell">
                        Purpose
                      </th>
                      <th className="text-left px-3 py-2.5 text-xs font-semibold text-muted-foreground uppercase tracking-wide hidden lg:table-cell">
                        Where to set
                      </th>
                      <th className="text-left px-3 py-2.5 text-xs font-semibold text-muted-foreground uppercase tracking-wide">
                        Value
                      </th>
                    </tr>
                  </thead>
                  <tbody>
                    {data.envVars.map((v, i) => (
                      <tr
                        key={v.key}
                        className={`border-t border-border ${
                          i % 2 === 0 ? "" : "bg-muted/20"
                        }`}
                      >
                        <td className="px-3 py-2.5">
                          <StatusIcon ok={v.set} />
                        </td>
                        <td className="px-3 py-2.5">
                          <span className="font-mono text-xs text-foreground">
                            {v.key}
                          </span>
                        </td>
                        <td className="px-3 py-2.5 hidden md:table-cell">
                          <Badge
                            variant={v.required ? "default" : "secondary"}
                            className="text-xs"
                          >
                            {v.required ? "Required" : "Optional"}
                          </Badge>
                        </td>
                        <td className="px-3 py-2.5 hidden lg:table-cell max-w-xs">
                          <span className="text-xs text-muted-foreground leading-tight">
                            {v.description}
                          </span>
                        </td>
                        <td className="px-3 py-2.5 hidden lg:table-cell">
                          <span className="text-xs text-muted-foreground/70 italic">
                            {v.setIn}
                          </span>
                        </td>
                        <td className="px-3 py-2.5">
                          {v.set ? (
                            <span className="font-mono text-xs text-muted-foreground">
                              {v.masked || "****"}
                            </span>
                          ) : (
                            <span className="text-xs text-muted-foreground/40 italic">
                              not set
                            </span>
                          )}
                        </td>
                      </tr>
                    ))}
                  </tbody>
                </table>
              </div>
              <p className="text-xs text-muted-foreground mt-2">
                Private keys and connection strings are never exposed — only their presence is confirmed.
              </p>
            </CardContent>
          </Card>
        </>
      )}
    </div>
  );
}
