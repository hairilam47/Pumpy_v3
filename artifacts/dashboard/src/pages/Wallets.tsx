import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  Wallet, CheckCircle, XCircle, PauseCircle, Play, Loader2,
  AlertTriangle, RefreshCw, Shield,
} from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { useToast } from "@/hooks/use-toast";
import { cn } from "@/lib/utils";

interface WalletEntry {
  walletId: string;
  status: string;
  ownerPubkey: string | null;
  lastActiveAt: string | null;
  createdAt: string | null;
}

interface WalletConfig {
  walletId: string;
  strategyPreset: string;
  status: string;
  riskPerTradeSol: number;
  dailyLossLimitSol: number;
}

function useWallets() {
  return useQuery<WalletEntry[]>({
    queryKey: ["wallets"],
    queryFn: async () => {
      const res = await fetch("/api/wallets");
      if (!res.ok) throw new Error("Failed to fetch wallets");
      return res.json() as Promise<WalletEntry[]>;
    },
    refetchInterval: 10_000,
  });
}

function useWalletConfig(walletId: string) {
  return useQuery<WalletConfig>({
    queryKey: ["walletConfig", walletId],
    queryFn: async () => {
      const res = await fetch(`/api/wallets/${walletId}/config`);
      if (!res.ok) throw new Error("Config not found");
      return res.json() as Promise<WalletConfig>;
    },
  });
}

function StatusBadge({ status }: { status: string }) {
  if (status === "enabled") {
    return (
      <span className="inline-flex items-center gap-1.5 px-2.5 py-0.5 rounded-full text-xs font-medium bg-green-500/15 text-green-400">
        <CheckCircle className="w-3 h-3" />
        enabled
      </span>
    );
  }
  if (status === "paused") {
    return (
      <span className="inline-flex items-center gap-1.5 px-2.5 py-0.5 rounded-full text-xs font-medium bg-yellow-500/15 text-yellow-400">
        <PauseCircle className="w-3 h-3" />
        paused
      </span>
    );
  }
  return (
    <span className="inline-flex items-center gap-1.5 px-2.5 py-0.5 rounded-full text-xs font-medium bg-red-500/15 text-red-400">
      <XCircle className="w-3 h-3" />
      halted
    </span>
  );
}

const PRESET_LABELS: Record<string, { label: string; color: string }> = {
  conservative: { label: "Conservative", color: "bg-blue-500/15 text-blue-400" },
  balanced: { label: "Balanced", color: "bg-primary/15 text-primary" },
  aggressive: { label: "Aggressive", color: "bg-orange-500/15 text-orange-400" },
};

function WalletCard({ wallet }: { wallet: WalletEntry }) {
  const { toast } = useToast();
  const qc = useQueryClient();
  const { data: config } = useWalletConfig(wallet.walletId);
  const [resuming, setResuming] = useState(false);

  const resumeMutation = useMutation({
    mutationFn: async () => {
      setResuming(true);
      const res = await fetch(`/api/wallets/${wallet.walletId}/resume`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({}),
      });
      if (!res.ok) {
        const err = (await res.json()) as { error: string };
        throw new Error(err.error ?? "Resume failed");
      }
      return res.json();
    },
    onSuccess: () => {
      toast({ title: `Wallet ${wallet.walletId} resumed` });
      void qc.invalidateQueries({ queryKey: ["wallets"] });
      void qc.invalidateQueries({ queryKey: ["walletConfig", wallet.walletId] });
    },
    onError: (err: Error) => {
      toast({
        title: "Resume failed",
        description: err.message,
        variant: "destructive",
      });
    },
    onSettled: () => setResuming(false),
  });

  const effectiveStatus = config?.status ?? wallet.status;
  const preset = config?.strategyPreset ?? "balanced";
  const presetMeta = PRESET_LABELS[preset] ?? PRESET_LABELS.balanced;
  const lastActive = wallet.lastActiveAt
    ? new Date(wallet.lastActiveAt).toLocaleString()
    : "Never";

  return (
    <Card className={cn(
      "transition-all",
      effectiveStatus === "halted" && "border-red-500/30",
      effectiveStatus === "paused" && "border-yellow-500/30",
      effectiveStatus === "enabled" && "border-green-500/20",
    )}>
      <CardHeader className="pb-3">
        <div className="flex items-center justify-between gap-2">
          <CardTitle className="text-base flex items-center gap-2 font-mono">
            <Wallet className="w-4 h-4 text-primary flex-shrink-0" />
            {wallet.walletId}
          </CardTitle>
          <StatusBadge status={effectiveStatus} />
        </div>
      </CardHeader>
      <CardContent className="space-y-3">
        {wallet.ownerPubkey && (
          <div className="rounded-lg bg-muted/40 px-3 py-2">
            <div className="text-xs text-muted-foreground mb-0.5">Public Key</div>
            <div className="font-mono text-xs text-foreground/80 break-all">
              {wallet.ownerPubkey}
            </div>
          </div>
        )}

        <div className="flex flex-wrap gap-2">
          <span className={cn("inline-flex items-center gap-1 px-2 py-0.5 rounded text-xs font-medium", presetMeta.color)}>
            <Shield className="w-3 h-3" />
            {presetMeta.label}
          </span>
          {config && (
            <span className="text-xs text-muted-foreground">
              Risk: {config.riskPerTradeSol} SOL/trade · Daily limit: {config.dailyLossLimitSol} SOL
            </span>
          )}
        </div>

        <div className="text-xs text-muted-foreground">
          Last active: {lastActive}
        </div>

        {effectiveStatus === "paused" && (
          <div className="rounded-lg bg-yellow-500/10 border border-yellow-500/20 px-3 py-2 flex items-start gap-2">
            <AlertTriangle className="w-4 h-4 text-yellow-400 flex-shrink-0 mt-0.5" />
            <div className="flex-1 min-w-0">
              <p className="text-xs text-yellow-400 font-medium">Wallet auto-paused</p>
              <p className="text-xs text-muted-foreground mt-0.5">
                Paused due to repeated decision engine rejections. Resume once the underlying issue is resolved.
              </p>
            </div>
            <Button
              size="sm"
              variant="outline"
              className="shrink-0 text-xs h-7 border-yellow-500/40 text-yellow-400 hover:bg-yellow-500/10"
              onClick={() => resumeMutation.mutate()}
              disabled={resuming || resumeMutation.isPending}
            >
              {resuming ? (
                <Loader2 className="w-3.5 h-3.5 animate-spin" />
              ) : (
                <Play className="w-3.5 h-3.5" />
              )}
              <span className="ml-1.5">Resume</span>
            </Button>
          </div>
        )}

        {effectiveStatus === "halted" && (
          <div className="rounded-lg bg-red-500/10 border border-red-500/20 px-3 py-2 flex items-start gap-2">
            <XCircle className="w-4 h-4 text-red-400 flex-shrink-0 mt-0.5" />
            <div className="text-xs text-red-400">
              <p className="font-medium">Wallet halted</p>
              <p className="text-muted-foreground mt-0.5">
                A critical execution error occurred. Contact your operator to re-enable via the admin CLI.
              </p>
            </div>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

export default function WalletsPage() {
  const { data: wallets, isLoading, error, refetch, isFetching } = useWallets();

  return (
    <div className="space-y-4 sm:space-y-6">
      <div className="flex items-start justify-between gap-2">
        <div>
          <h1 className="text-xl sm:text-2xl font-bold text-foreground flex items-center gap-2">
            <Wallet className="w-5 h-5 sm:w-6 sm:h-6 text-primary flex-shrink-0" />
            Wallets
          </h1>
          <p className="text-sm text-muted-foreground mt-1">
            Monitor wallet health and resume paused wallets.
          </p>
        </div>
        <Button
          variant="outline"
          size="sm"
          onClick={() => void refetch()}
          disabled={isFetching}
          className="shrink-0"
        >
          {isFetching ? (
            <Loader2 className="w-3.5 h-3.5 animate-spin" />
          ) : (
            <RefreshCw className="w-3.5 h-3.5" />
          )}
          <span className="ml-1.5 hidden sm:inline">Refresh</span>
        </Button>
      </div>

      {isLoading && (
        <div className="text-sm text-muted-foreground animate-pulse">Loading wallets…</div>
      )}

      {error && (
        <div className="flex items-center gap-2 text-sm text-destructive">
          <AlertTriangle className="w-4 h-4" />
          Failed to load wallets — API server may be offline.
        </div>
      )}

      {wallets && wallets.length === 0 && (
        <Card>
          <CardContent className="py-8 text-center">
            <Wallet className="w-10 h-10 text-muted-foreground mx-auto mb-3" />
            <p className="text-sm font-medium text-foreground/80">No wallets registered</p>
            <p className="text-xs text-muted-foreground mt-1">
              Ask your operator to register a wallet via the admin CLI or set{" "}
              <span className="font-mono">KEYPAIR_PATH</span> so the engine can auto-register one.
            </p>
          </CardContent>
        </Card>
      )}

      {wallets && wallets.length > 0 && (
        <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
          {wallets.map((w) => (
            <WalletCard key={w.walletId} wallet={w} />
          ))}
        </div>
      )}

      <div className="rounded-lg bg-muted/30 border border-border/40 px-4 py-3 text-xs text-muted-foreground space-y-1">
        <p className="font-medium text-foreground/70">Wallet lifecycle</p>
        <p>
          <span className="text-green-400 font-medium">enabled</span> — active and processing orders.
        </p>
        <p>
          <span className="text-yellow-400 font-medium">paused</span> — auto-paused after{" "}
          repeated rejections; orders are blocked until resumed.
        </p>
        <p>
          <span className="text-red-400 font-medium">halted</span> — stopped after a critical
          execution error; requires operator intervention.
        </p>
      </div>
    </div>
  );
}
