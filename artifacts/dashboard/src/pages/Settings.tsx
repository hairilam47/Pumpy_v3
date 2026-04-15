import { useState, useEffect } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  CheckCircle, XCircle, AlertCircle, Wifi, WifiOff, Key, Server,
  Settings2, AlertTriangle, Save, Loader2, FlaskConical, ChevronDown, ChevronUp,
} from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useToast } from "@/hooks/use-toast";

interface EnvVar {
  key: string;
  required: boolean;
  description: string;
  setIn: string;
  set: boolean;
  source: "db" | "env" | null;
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

type ConfigMap = Record<string, string>;

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

function useSettingsConfig() {
  return useQuery<ConfigMap>({
    queryKey: ["settingsConfig"],
    queryFn: async () => {
      const res = await fetch("/api/settings/config");
      if (!res.ok) throw new Error("Failed to fetch config");
      return res.json() as Promise<ConfigMap>;
    },
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

function RestartNotice() {
  return (
    <div className="flex items-start gap-2 rounded-lg bg-yellow-500/10 border border-yellow-500/20 px-3 py-2.5 text-xs text-yellow-400">
      <AlertTriangle className="w-3.5 h-3.5 mt-0.5 flex-shrink-0" />
      <span>
        Changes saved. <strong>Restart the trading engine</strong> for connection or risk settings to take effect.
      </span>
    </div>
  );
}

// ── Connection Card ───────────────────────────────────────────────────────────

function ConnectionCard({ config }: { config: ConfigMap }) {
  const { toast } = useToast();
  const queryClient = useQueryClient();

  const [rpcUrl, setRpcUrl] = useState(config["SOLANA_RPC_URL"] ?? "");
  const [rpcUrls, setRpcUrls] = useState(config["SOLANA_RPC_URLS"] ?? "");
  const [jitoUrl, setJitoUrl] = useState(config["JITO_BUNDLE_URL"] ?? "");
  const [isSaving, setIsSaving] = useState(false);
  const [isTesting, setIsTesting] = useState(false);
  const [testResult, setTestResult] = useState<{ ok: boolean; latencyMs: number | null } | null>(null);
  const [showRestart, setShowRestart] = useState(false);

  useEffect(() => {
    setRpcUrl(config["SOLANA_RPC_URL"] ?? "");
    setRpcUrls(config["SOLANA_RPC_URLS"] ?? "");
    setJitoUrl(config["JITO_BUNDLE_URL"] ?? "");
  }, [config]);

  async function handleTestRpc() {
    if (!rpcUrl.trim()) {
      toast({ title: "Enter an RPC URL first", variant: "destructive" });
      return;
    }
    setIsTesting(true);
    setTestResult(null);
    try {
      const res = await fetch("/api/settings/config/test-rpc", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ url: rpcUrl.trim() }),
      });
      const data = (await res.json()) as { ok: boolean; latencyMs: number | null };
      setTestResult(data);
    } catch {
      setTestResult({ ok: false, latencyMs: null });
    } finally {
      setIsTesting(false);
    }
  }

  async function handleSave() {
    setIsSaving(true);
    try {
      const res = await fetch("/api/settings/config", {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          SOLANA_RPC_URL: rpcUrl.trim(),
          SOLANA_RPC_URLS: rpcUrls.trim(),
          JITO_BUNDLE_URL: jitoUrl.trim(),
        }),
      });
      if (!res.ok) throw new Error("Save failed");
      toast({ title: "Connection settings saved" });
      setShowRestart(true);
      void queryClient.invalidateQueries({ queryKey: ["settingsConfig"] });
      void queryClient.invalidateQueries({ queryKey: ["settingsStatus"] });
    } catch {
      toast({ title: "Failed to save settings", variant: "destructive" });
    } finally {
      setIsSaving(false);
    }
  }

  return (
    <Card>
      <CardHeader className="pb-3">
        <CardTitle className="text-base flex items-center gap-2">
          <Wifi className="w-4 h-4 text-primary" />
          Connection
        </CardTitle>
      </CardHeader>
      <CardContent className="space-y-4">
        {showRestart && <RestartNotice />}

        <div className="space-y-1.5">
          <Label htmlFor="rpc-url" className="text-xs">
            Primary RPC URL
            <span className="ml-1.5 text-muted-foreground font-normal">(SOLANA_RPC_URL)</span>
          </Label>
          <div className="flex gap-2">
            <Input
              id="rpc-url"
              value={rpcUrl}
              onChange={(e) => { setRpcUrl(e.target.value); setTestResult(null); }}
              placeholder="https://your-rpc.helius.xyz/..."
              className="font-mono text-xs flex-1"
            />
            <Button
              variant="outline"
              size="sm"
              onClick={handleTestRpc}
              disabled={isTesting || !rpcUrl.trim()}
              className="shrink-0"
            >
              {isTesting ? (
                <Loader2 className="w-3.5 h-3.5 animate-spin" />
              ) : (
                <FlaskConical className="w-3.5 h-3.5" />
              )}
              <span className="ml-1.5">Test</span>
            </Button>
          </div>
          {testResult && (
            <div
              className={`flex items-center gap-2 text-xs mt-1 ${
                testResult.ok ? "text-green-400" : "text-destructive"
              }`}
            >
              <StatusIcon ok={testResult.ok} />
              {testResult.ok
                ? `Connected — ${testResult.latencyMs}ms`
                : "Connection failed — check the URL"}
            </div>
          )}
        </div>

        <div className="space-y-1.5">
          <Label htmlFor="rpc-urls" className="text-xs">
            Failover RPC URLs
            <span className="ml-1.5 text-muted-foreground font-normal">
              (SOLANA_RPC_URLS — comma-separated, optional)
            </span>
          </Label>
          <Input
            id="rpc-urls"
            value={rpcUrls}
            onChange={(e) => setRpcUrls(e.target.value)}
            placeholder="https://backup1.rpc.com,https://backup2.rpc.com"
            className="font-mono text-xs"
          />
        </div>

        <div className="space-y-1.5">
          <Label htmlFor="jito-url" className="text-xs">
            Jito Bundle URL
            <span className="ml-1.5 text-muted-foreground font-normal">
              (JITO_BUNDLE_URL — optional, enables MEV protection)
            </span>
          </Label>
          <Input
            id="jito-url"
            value={jitoUrl}
            onChange={(e) => setJitoUrl(e.target.value)}
            placeholder="https://mainnet.block-engine.jito.wtf/api/v1/bundles"
            className="font-mono text-xs"
          />
        </div>

        <Button onClick={handleSave} disabled={isSaving} size="sm">
          {isSaving ? (
            <Loader2 className="w-3.5 h-3.5 animate-spin mr-1.5" />
          ) : (
            <Save className="w-3.5 h-3.5 mr-1.5" />
          )}
          Save connection settings
        </Button>
      </CardContent>
    </Card>
  );
}

// ── Trading Parameters Card ───────────────────────────────────────────────────

function TradingParametersCard({ config }: { config: ConfigMap }) {
  const { toast } = useToast();
  const queryClient = useQueryClient();

  const [maxPos, setMaxPos] = useState(config["MAX_POSITION_SIZE_SOL"] ?? "");
  const [stopLoss, setStopLoss] = useState(config["STOP_LOSS_PERCENT"] ?? "");
  const [takeProfit, setTakeProfit] = useState(config["TAKE_PROFIT_PERCENT"] ?? "");
  const [isSaving, setIsSaving] = useState(false);
  const [showRestart, setShowRestart] = useState(false);

  useEffect(() => {
    setMaxPos(config["MAX_POSITION_SIZE_SOL"] ?? "");
    setStopLoss(config["STOP_LOSS_PERCENT"] ?? "");
    setTakeProfit(config["TAKE_PROFIT_PERCENT"] ?? "");
  }, [config]);

  async function handleSave() {
    setIsSaving(true);
    try {
      const res = await fetch("/api/settings/config", {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          MAX_POSITION_SIZE_SOL: maxPos.trim(),
          STOP_LOSS_PERCENT: stopLoss.trim(),
          TAKE_PROFIT_PERCENT: takeProfit.trim(),
        }),
      });
      if (!res.ok) throw new Error("Save failed");
      toast({ title: "Trading parameters saved" });
      setShowRestart(true);
      void queryClient.invalidateQueries({ queryKey: ["settingsConfig"] });
    } catch {
      toast({ title: "Failed to save parameters", variant: "destructive" });
    } finally {
      setIsSaving(false);
    }
  }

  return (
    <Card>
      <CardHeader className="pb-3">
        <CardTitle className="text-base flex items-center gap-2">
          <Settings2 className="w-4 h-4 text-primary" />
          Trading Parameters
        </CardTitle>
      </CardHeader>
      <CardContent className="space-y-4">
        {showRestart && <RestartNotice />}

        <div className="grid grid-cols-1 sm:grid-cols-3 gap-4">
          <div className="space-y-1.5">
            <Label htmlFor="max-pos" className="text-xs">
              Max Position Size (SOL)
            </Label>
            <Input
              id="max-pos"
              type="number"
              step="0.1"
              min="0"
              value={maxPos}
              onChange={(e) => setMaxPos(e.target.value)}
              placeholder="10"
              className="font-mono text-xs"
            />
            <p className="text-xs text-muted-foreground">Max SOL per trade</p>
          </div>

          <div className="space-y-1.5">
            <Label htmlFor="stop-loss" className="text-xs">
              Stop-Loss (%)
            </Label>
            <Input
              id="stop-loss"
              type="number"
              step="1"
              min="0"
              max="100"
              value={stopLoss}
              onChange={(e) => setStopLoss(e.target.value)}
              placeholder="10"
              className="font-mono text-xs"
            />
            <p className="text-xs text-muted-foreground">Exit at −X%</p>
          </div>

          <div className="space-y-1.5">
            <Label htmlFor="take-profit" className="text-xs">
              Take-Profit (%)
            </Label>
            <Input
              id="take-profit"
              type="number"
              step="1"
              min="0"
              value={takeProfit}
              onChange={(e) => setTakeProfit(e.target.value)}
              placeholder="50"
              className="font-mono text-xs"
            />
            <p className="text-xs text-muted-foreground">Exit at +X%</p>
          </div>
        </div>

        <p className="text-xs text-muted-foreground/70">
          Note: Stop-Loss and Take-Profit are persisted and reserved for the Python strategy engine. Max Position Size is applied to the Rust execution engine on next restart.
        </p>

        <Button onClick={handleSave} disabled={isSaving} size="sm">
          {isSaving ? (
            <Loader2 className="w-3.5 h-3.5 animate-spin mr-1.5" />
          ) : (
            <Save className="w-3.5 h-3.5 mr-1.5" />
          )}
          Save parameters
        </Button>
      </CardContent>
    </Card>
  );
}

// ── Service URLs Card ─────────────────────────────────────────────────────────

function ServiceUrlsCard({ config }: { config: ConfigMap }) {
  const { toast } = useToast();
  const queryClient = useQueryClient();

  const [grpcUrl, setGrpcUrl] = useState(config["RUST_GRPC_URL"] ?? "");
  const [pythonUrl, setPythonUrl] = useState(config["PYTHON_STRATEGY_URL"] ?? "");
  const [isSaving, setIsSaving] = useState(false);

  useEffect(() => {
    setGrpcUrl(config["RUST_GRPC_URL"] ?? "");
    setPythonUrl(config["PYTHON_STRATEGY_URL"] ?? "");
  }, [config]);

  async function handleSave() {
    setIsSaving(true);
    try {
      const res = await fetch("/api/settings/config", {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          RUST_GRPC_URL: grpcUrl.trim(),
          PYTHON_STRATEGY_URL: pythonUrl.trim(),
        }),
      });
      if (!res.ok) throw new Error("Save failed");
      toast({ title: "Service URLs saved" });
      void queryClient.invalidateQueries({ queryKey: ["settingsConfig"] });
    } catch {
      toast({ title: "Failed to save service URLs", variant: "destructive" });
    } finally {
      setIsSaving(false);
    }
  }

  return (
    <Card>
      <CardHeader className="pb-3">
        <CardTitle className="text-base flex items-center gap-2">
          <Server className="w-4 h-4 text-primary" />
          Internal Service URLs
        </CardTitle>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="space-y-1.5">
          <Label htmlFor="grpc-url" className="text-xs">
            Rust gRPC Address
            <span className="ml-1.5 text-muted-foreground font-normal">(RUST_GRPC_URL)</span>
          </Label>
          <Input
            id="grpc-url"
            value={grpcUrl}
            onChange={(e) => setGrpcUrl(e.target.value)}
            placeholder="localhost:50051"
            className="font-mono text-xs"
          />
        </div>

        <div className="space-y-1.5">
          <Label htmlFor="python-url" className="text-xs">
            Python Strategy Engine URL
            <span className="ml-1.5 text-muted-foreground font-normal">(PYTHON_STRATEGY_URL)</span>
          </Label>
          <Input
            id="python-url"
            value={pythonUrl}
            onChange={(e) => setPythonUrl(e.target.value)}
            placeholder="http://localhost:8001"
            className="font-mono text-xs"
          />
        </div>

        <Button onClick={handleSave} disabled={isSaving} size="sm">
          {isSaving ? (
            <Loader2 className="w-3.5 h-3.5 animate-spin mr-1.5" />
          ) : (
            <Save className="w-3.5 h-3.5 mr-1.5" />
          )}
          Save service URLs
        </Button>
      </CardContent>
    </Card>
  );
}

// ── Wallet Setup Guide ────────────────────────────────────────────────────────

function Code({ children }: { children: React.ReactNode }) {
  return (
    <code className="font-mono bg-secondary/60 rounded px-1 py-0.5 text-[11px] text-foreground/90 break-all">
      {children}
    </code>
  );
}

function WalletSetupGuide() {
  const [open, setOpen] = useState(false);

  return (
    <div className="mt-3 rounded-lg border border-dashed border-border/60">
      <button
        onClick={() => setOpen((v) => !v)}
        className="flex w-full items-center justify-between px-3 py-2 text-xs text-muted-foreground hover:text-foreground transition-colors"
      >
        <span className="font-medium">Show setup guide</span>
        {open ? (
          <ChevronUp className="w-3.5 h-3.5 flex-shrink-0" />
        ) : (
          <ChevronDown className="w-3.5 h-3.5 flex-shrink-0" />
        )}
      </button>

      <div
        className={`overflow-hidden transition-all duration-300 ease-in-out ${
          open ? "max-h-[600px] opacity-100" : "max-h-0 opacity-0"
        }`}
      >
        <div className="px-3 pb-4 space-y-4 text-xs text-foreground/80">
          <div className="h-px bg-border/40" />
          <p className="font-semibold text-sm text-foreground/90">How to set up your wallet</p>

          {/* Step 1 */}
          <div className="space-y-1.5">
            <div className="flex items-start gap-2">
              <span className="flex-shrink-0 w-5 h-5 rounded-full bg-primary/15 text-primary flex items-center justify-center text-[10px] font-bold">
                1
              </span>
              <p className="font-semibold text-foreground/90">Generate a Solana keypair</p>
            </div>
            <p className="ml-7 text-muted-foreground">
              If you have the Solana CLI installed, run:
            </p>
            <div className="ml-7 rounded bg-muted/50 px-3 py-2 font-mono text-[11px] text-foreground/80">
              solana-keygen new --outfile ~/my-wallet.json
            </div>
            <p className="ml-7 text-muted-foreground">
              Or generate one online at{" "}
              <span className="text-primary">keypair.solana.com</span> and save the JSON file.
            </p>
          </div>

          {/* Step 2 */}
          <div className="space-y-1.5">
            <div className="flex items-start gap-2">
              <span className="flex-shrink-0 w-5 h-5 rounded-full bg-primary/15 text-primary flex items-center justify-center text-[10px] font-bold">
                2
              </span>
              <p className="font-semibold text-foreground/90">Get the private key bytes</p>
            </div>
            <p className="ml-7 text-muted-foreground">
              The keypair file contains a JSON array of 64 numbers — this is what you need.
              View it with:
            </p>
            <div className="ml-7 rounded bg-muted/50 px-3 py-2 font-mono text-[11px] text-foreground/80">
              cat ~/my-wallet.json
            </div>
            <div className="ml-7 rounded bg-yellow-500/10 border border-yellow-500/20 px-3 py-2 text-yellow-400 space-y-1">
              <p className="font-semibold">Important:</p>
              <p>
                The output of <Code>solana-keygen pubkey</Code> is the <em>public key only</em> —
                not usable here. You need the full 64-byte JSON array from the keypair file.
              </p>
            </div>
            <p className="ml-7 text-muted-foreground">
              Alternatively, paste your base58-encoded private key (64 bytes = 88 base58 chars).
            </p>
          </div>

          {/* Step 3 */}
          <div className="space-y-1.5">
            <div className="flex items-start gap-2">
              <span className="flex-shrink-0 w-5 h-5 rounded-full bg-primary/15 text-primary flex items-center justify-center text-[10px] font-bold">
                3
              </span>
              <p className="font-semibold text-foreground/90">Add it to Replit Secrets</p>
            </div>
            <ol className="ml-7 text-muted-foreground space-y-1 list-decimal list-inside">
              <li>Click the <strong className="text-foreground/80">padlock icon</strong> in the Replit left sidebar (Secrets)</li>
              <li>Click <strong className="text-foreground/80">New secret</strong></li>
              <li>
                Set the key to <Code>WALLET_PRIVATE_KEY</Code>
              </li>
              <li>
                Paste the JSON array (e.g. <Code>[12,34,56,…]</Code>) as the value
              </li>
              <li>Click <strong className="text-foreground/80">Add secret</strong></li>
            </ol>
          </div>

          {/* Step 4 */}
          <div className="space-y-1.5">
            <div className="flex items-start gap-2">
              <span className="flex-shrink-0 w-5 h-5 rounded-full bg-primary/15 text-primary flex items-center justify-center text-[10px] font-bold">
                4
              </span>
              <p className="font-semibold text-foreground/90">Restart the Trading Engine</p>
            </div>
            <p className="ml-7 text-muted-foreground">
              Click the <strong className="text-foreground/80">Restart</strong> button next to the{" "}
              <Code>rust-engine: Trading Engine</Code> workflow in the Replit toolbar.
              The wallet badge above will turn green and the Dashboard "Rust Engine" indicator will go live.
            </p>
          </div>
        </div>
      </div>
    </div>
  );
}

// ── Main Page ─────────────────────────────────────────────────────────────────

export default function SettingsPage() {
  const { data: status, isLoading, error, dataUpdatedAt } = useSettingsStatus();
  const { data: config, isLoading: configLoading } = useSettingsConfig();

  const lastUpdated = dataUpdatedAt ? new Date(dataUpdatedAt).toLocaleTimeString() : null;

  return (
    <div className="space-y-4 sm:space-y-6">
      {/* Header */}
      <div className="space-y-1">
        <div className="flex items-start justify-between gap-2">
          <h1 className="text-xl sm:text-2xl font-bold text-foreground flex items-center gap-2">
            <Settings2 className="w-5 h-5 sm:w-6 sm:h-6 text-primary flex-shrink-0" />
            Settings
          </h1>
          {lastUpdated && (
            <p className="text-xs text-muted-foreground whitespace-nowrap mt-1">
              Updated {lastUpdated}
            </p>
          )}
        </div>
        <p className="text-sm text-muted-foreground">
          Wallet keys must be set in Replit Secrets. All other settings are editable here.
        </p>
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

      {/* Wallet — read-only */}
      {status && (
        <Card>
          <CardHeader className="pb-3">
            <CardTitle className="text-base flex flex-wrap items-center gap-2">
              <Key className="w-4 h-4 text-primary flex-shrink-0" />
              Wallet
              <Badge variant="secondary" className="text-xs ml-auto font-normal">Read-only</Badge>
            </CardTitle>
          </CardHeader>
          <CardContent className="space-y-3">
            <div className="flex items-center gap-3">
              <StatusIcon ok={status.wallet.configured} />
              <div className="flex-1">
                <div className="text-sm font-medium text-foreground">
                  {status.wallet.configured ? "Wallet configured" : "No wallet configured"}
                </div>
                {status.wallet.source && (
                  <div className="text-xs text-muted-foreground mt-0.5">
                    Source: <span className="font-mono">{status.wallet.source}</span>
                  </div>
                )}
              </div>
              {status.wallet.configured && (
                <Badge variant="secondary" className="text-xs">
                  {status.wallet.source === "WALLET_PRIVATE_KEY" ? "Env Secret" : "Key File"}
                </Badge>
              )}
            </div>

            {status.wallet.pubkey ? (
              <div className="rounded-lg bg-muted/50 px-3 py-2.5">
                <div className="text-xs text-muted-foreground mb-1">Public Key</div>
                <div className="font-mono text-sm text-foreground break-all">
                  {status.wallet.pubkey}
                </div>
              </div>
            ) : status.wallet.configured ? (
              <div className="rounded-lg bg-muted/30 px-3 py-2 text-xs text-muted-foreground">
                Public key is available when the Rust engine starts, or when{" "}
                <span className="font-mono">WALLET_PRIVATE_KEY</span> is set as a JSON byte array.
              </div>
            ) : (
              <>
                <div className="rounded-lg bg-destructive/10 px-3 py-2 text-xs text-destructive">
                  Set <span className="font-mono">WALLET_PRIVATE_KEY</span> (base58 or JSON array)
                  in the <span className="font-semibold">Replit Secrets panel</span>, or set{" "}
                  <span className="font-mono">KEYPAIR_PATH</span> to a keypair file path.
                </div>
                <WalletSetupGuide />
              </>
            )}
          </CardContent>
        </Card>
      )}

      {/* Editable sections — populated once config loads */}
      {config && !configLoading && (
        <>
          <ConnectionCard config={config} />
          <TradingParametersCard config={config} />
          <ServiceUrlsCard config={config} />
        </>
      )}

      {configLoading && (
        <div className="flex items-center gap-2 text-sm text-muted-foreground animate-pulse">
          <Loader2 className="w-4 h-4 animate-spin" />
          Loading saved config…
        </div>
      )}

      {/* DATABASE_URL — read-only status */}
      {status && (
        <Card>
          <CardHeader className="pb-3">
            <CardTitle className="text-base flex items-center gap-2">
              <Server className="w-4 h-4 text-primary" />
              Infrastructure
              <Badge variant="secondary" className="text-xs ml-auto font-normal">Read-only</Badge>
            </CardTitle>
          </CardHeader>
          <CardContent>
            {status.envVars
              .filter((v) => ["DATABASE_URL", "GRPC_PORT", "METRICS_PORT"].includes(v.key))
              .map((v) => (
                <div key={v.key} className="flex items-center gap-2 py-2 border-b border-border last:border-0">
                  <StatusIcon ok={v.set} />
                  <span className="font-mono text-xs text-foreground flex-1 min-w-0 truncate">{v.key}</span>
                  <span className="text-xs text-muted-foreground truncate max-w-[100px] sm:max-w-[200px]">
                    {v.set ? v.masked || "****" : <span className="italic opacity-40">not set</span>}
                  </span>
                  {v.set && (
                    <Badge variant="secondary" className="text-xs">
                      {v.source === "db" ? "saved" : "env"}
                    </Badge>
                  )}
                </div>
              ))}
            <p className="text-xs text-muted-foreground mt-3">
              DATABASE_URL is auto-injected by the Replit database attachment. Private keys are never exposed here.
            </p>
          </CardContent>
        </Card>
      )}
    </div>
  );
}
