import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Shield, Loader2, AlertTriangle, CheckCircle2, Zap, TrendingUp, KeyRound, X } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { cn } from "@/lib/utils";
import { useToast } from "@/hooks/use-toast";
import { useAdminKey } from "@/hooks/use-admin-key";

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
  const [adminKey, setAdminKey] = useAdminKey();
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
    onSuccess: (_data, { preset }) => {
      toast({ title: `Preset changed to ${preset} for ${wallet.walletId}` });
      setPendingPreset(null);
      if (!rememberKey) setAdminKey("");
      void qc.invalidateQueries({ queryKey: ["walletConfig", wallet.walletId] });
    },
    onError: (err: Error) => {
      toast({ title: "Failed to change preset", description: err.message, variant: "destructive" });
      setAdminKey("");
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
                      setPendingPreset(preset.id);
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

          {/* Admin key prompt — shown when a non-active preset is selected */}
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
                    if (e.key === "Escape") { setPendingPreset(null); setAdminKey(""); setRememberKey(false); }
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
                  onClick={() => { setPendingPreset(null); setAdminKey(""); setRememberKey(false); }}
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
    </div>
  );
}
