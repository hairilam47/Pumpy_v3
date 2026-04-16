import { useState, useEffect } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import {
  CheckCircle, XCircle, AlertCircle, Wifi, WifiOff, Key, Server,
  Settings2, AlertTriangle, Save, Loader2, FlaskConical, Shield,
  Info, X, Zap,
} from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { Checkbox } from "@/components/ui/checkbox";
import { useToast } from "@/hooks/use-toast";
import { useAdminKey, getValidAdminKey } from "@/hooks/use-admin-key";
import { cn } from "@/lib/utils";

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

function useActivePreset() {
  return useQuery<{ preset: string }>({
    queryKey: ["activePreset"],
    queryFn: async () => {
      const res = await fetch("/api/wallets/wallet_001/config");
      if (!res.ok) return { preset: "balanced" };
      const data = (await res.json()) as { strategyPreset?: string };
      return { preset: data.strategyPreset ?? "balanced" };
    },
    staleTime: 10_000,
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

// ── Strategy Preset Selector ──────────────────────────────────────────────────

const PRESETS = [
  {
    id: "conservative",
    label: "Conservative",
    description: "Small trades, tight stop-loss. Lower risk, lower reward.",
    detail: "0.05 SOL/trade · 5% stop · 20% target · max 2 positions",
    color: "border-blue-500/40 bg-blue-500/5 text-blue-400",
    activeColor: "border-blue-500 bg-blue-500/15 text-blue-300",
  },
  {
    id: "balanced",
    label: "Balanced",
    description: "Moderate sizing with sensible risk controls. Good starting point.",
    detail: "0.15 SOL/trade · 10% stop · 50% target · max 5 positions",
    color: "border-primary/30 bg-primary/5 text-primary/80",
    activeColor: "border-primary bg-primary/15 text-primary",
  },
  {
    id: "aggressive",
    label: "Aggressive",
    description: "Larger positions, wider stops. Higher risk, higher reward.",
    detail: "0.5 SOL/trade · 20% stop · 100% target · max 10 positions",
    color: "border-orange-500/30 bg-orange-500/5 text-orange-400/80",
    activeColor: "border-orange-500 bg-orange-500/15 text-orange-300",
  },
] as const;

type PresetId = (typeof PRESETS)[number]["id"];

function StrategyPresetCard() {
  const { toast } = useToast();
  const qc = useQueryClient();
  const { data: presetData } = useActivePreset();
  const [pendingPreset, setPendingPreset] = useState<PresetId | null>(null);
  const { adminKey, setAdminKey, rememberAdminKey, clearAdminKey } = useAdminKey();
  const [rememberKey, setRememberKey] = useState(() => adminKey.length > 0);

  const activePreset = (presetData?.preset ?? "balanced") as PresetId;

  const savePreset = useMutation({
    mutationFn: async ({ preset, key }: { preset: PresetId; key: string }) => {
      const res = await fetch("/api/strategy/preset", {
        method: "PUT",
        headers: {
          "Content-Type": "application/json",
          "X-Admin-Key": key,
        },
        body: JSON.stringify({ preset }),
      });
      if (!res.ok) {
        const err = (await res.json()) as { error?: string; detail?: string };
        throw new Error(err.error ?? err.detail ?? "Failed to save preset");
      }
      return res.json();
    },
    onSuccess: (_data, { preset, key }) => {
      toast({ title: `Strategy preset set to ${preset}` });
      void qc.invalidateQueries({ queryKey: ["activePreset"] });
      setPendingPreset(null);
      if (rememberKey) {
        rememberAdminKey(key);
      } else {
        clearAdminKey();
      }
    },
    onError: (err: Error) => {
      toast({ title: "Failed to save preset", description: err.message, variant: "destructive" });
      clearAdminKey();
      setRememberKey(false);
    },
  });

  return (
    <Card>
      <CardHeader className="pb-3">
        <CardTitle className="text-base flex items-center gap-2">
          <Shield className="w-4 h-4 text-primary" />
          Strategy Preset
        </CardTitle>
      </CardHeader>
      <CardContent className="space-y-3">
        <p className="text-xs text-muted-foreground">
          Choose a risk profile for your trading strategies. The preset controls position size,
          stop-loss, take-profit, and max concurrent positions.
        </p>

        <div className="grid grid-cols-1 sm:grid-cols-3 gap-3">
          {PRESETS.map((p) => {
            const isActive = p.id === activePreset;
            const isPending = pendingPreset === p.id;
            return (
              <button
                key={p.id}
                onClick={() => {
                  if (!isActive && !pendingPreset) {
                    const key = getValidAdminKey();
                    if (key) {
                      savePreset.mutate({ preset: p.id, key });
                    } else {
                      setPendingPreset(p.id);
                    }
                  }
                }}
                disabled={savePreset.isPending || !!pendingPreset}
                className={cn(
                  "relative rounded-lg border-2 px-3 py-3 text-left transition-all",
                  isActive ? p.activeColor : p.color,
                  !pendingPreset && !isActive && "hover:opacity-90 cursor-pointer",
                  isPending && "ring-2 ring-primary/60",
                  !!pendingPreset && !isPending && !isActive && "opacity-50",
                )}
              >
                {isActive && !isPending && (
                  <span className="absolute top-2 right-2">
                    <CheckCircle className="w-3.5 h-3.5" />
                  </span>
                )}
                {isPending && savePreset.isPending && (
                  <span className="absolute top-2 right-2">
                    <Loader2 className="w-3.5 h-3.5 animate-spin" />
                  </span>
                )}
                <div className="font-semibold text-sm mb-1">{p.label}</div>
                <div className="text-xs opacity-80 leading-relaxed mb-2">{p.description}</div>
                <div className="text-[10px] font-mono opacity-60">{p.detail}</div>
              </button>
            );
          })}
        </div>

        {/* Auto-applying indicator — shown when cached key fires without a prompt */}
        {savePreset.isPending && !pendingPreset && (
          <div className="flex items-center gap-2 text-xs text-muted-foreground rounded-lg bg-primary/5 px-3 py-2">
            <Loader2 className="w-3.5 h-3.5 animate-spin flex-shrink-0 text-primary" />
            Applying with remembered key…
          </div>
        )}

        {/* Admin key prompt — shown when a non-active preset is selected and no key is cached */}
        {pendingPreset && (
          <div className="rounded-lg border border-primary/30 bg-primary/5 px-3 py-2 space-y-2">
            <p className="text-xs text-muted-foreground">
              Changing to{" "}
              <span className="font-semibold text-foreground capitalize">{pendingPreset}</span>{" "}
              preset requires admin authorization.
            </p>
            <div className="flex items-center gap-2">
              <Key className="w-4 h-4 text-primary flex-shrink-0" />
              <Input
                type="password"
                placeholder="Enter admin key…"
                value={adminKey}
                onChange={(e) => setAdminKey(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && adminKey) savePreset.mutate({ preset: pendingPreset, key: adminKey });
                  if (e.key === "Escape") { setPendingPreset(null); clearAdminKey(); setRememberKey(false); }
                }}
                className="h-7 text-xs flex-1"
                autoFocus
              />
              <Button
                size="sm"
                className="h-7 text-xs"
                onClick={() => savePreset.mutate({ preset: pendingPreset, key: adminKey })}
                disabled={!adminKey || savePreset.isPending}
              >
                {savePreset.isPending ? <Loader2 className="w-3.5 h-3.5 animate-spin" /> : "Apply"}
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
            <label className="flex items-center gap-2 text-xs text-muted-foreground cursor-pointer select-none">
              <Checkbox
                id="remember-key-settings"
                checked={rememberKey}
                onCheckedChange={(c) => setRememberKey(c === true)}
              />
              Remember for 1 hour
            </label>
          </div>
        )}

        <div className="flex items-start gap-2 rounded-lg bg-muted/30 px-3 py-2 text-xs text-muted-foreground">
          <Info className="w-3.5 h-3.5 mt-0.5 flex-shrink-0" />
          <span>
            The preset is applied by the Python strategy engine. Changes take effect on the next
            strategy evaluation cycle — no engine restart required.
          </span>
        </div>
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

// ── Jito MEV Settings Card ────────────────────────────────────────────────────

function JitoMevSettingsCard({ config }: { config: ConfigMap }) {
  const { toast } = useToast();
  const queryClient = useQueryClient();

  const [tipPercent, setTipPercent] = useState(config["JITO_TIP_PERCENT"] ?? "0.001");
  const [tipFloor, setTipFloor] = useState(config["JITO_TIP_FLOOR_LAMPORTS"] ?? "5000");
  const [tipCeiling, setTipCeiling] = useState(config["JITO_TIP_CEILING_LAMPORTS"] ?? "10000000");
  const [simEnabled, setSimEnabled] = useState(
    (config["JITO_SIMULATION_ENABLED"] ?? "true") === "true"
  );
  const [isSaving, setIsSaving] = useState(false);
  const [savedOk, setSavedOk] = useState(false);

  useEffect(() => {
    setTipPercent(config["JITO_TIP_PERCENT"] ?? "0.001");
    setTipFloor(config["JITO_TIP_FLOOR_LAMPORTS"] ?? "5000");
    setTipCeiling(config["JITO_TIP_CEILING_LAMPORTS"] ?? "10000000");
    setSimEnabled((config["JITO_SIMULATION_ENABLED"] ?? "true") === "true");
  }, [config]);

  function lamportsToSol(lamports: string): string {
    const n = parseFloat(lamports);
    if (isNaN(n) || n < 0) return "—";
    const sol = n / 1e9;
    if (sol === 0) return "0 SOL";
    if (sol < 0.000001) return `${sol.toExponential(2)} SOL`;
    return `${sol.toFixed(9).replace(/\.?0+$/, "")} SOL`;
  }

  function tipPercentDisplay(val: string): string {
    const n = parseFloat(val);
    if (isNaN(n) || n < 0) return "—";
    const pct = n * 100;
    return `${pct < 0.001 ? pct.toExponential(2) : pct.toFixed(4).replace(/\.?0+$/, "")}%`;
  }

  async function handleSave() {
    setIsSaving(true);
    setSavedOk(false);
    try {
      const res = await fetch("/api/settings/config", {
        method: "PUT",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          JITO_TIP_PERCENT: tipPercent.trim(),
          JITO_TIP_FLOOR_LAMPORTS: tipFloor.trim(),
          JITO_TIP_CEILING_LAMPORTS: tipCeiling.trim(),
          JITO_SIMULATION_ENABLED: simEnabled ? "true" : "false",
        }),
      });
      if (!res.ok) throw new Error("Save failed");
      toast({ title: "Jito MEV settings saved" });
      setSavedOk(true);
      void queryClient.invalidateQueries({ queryKey: ["settingsConfig"] });
    } catch {
      toast({ title: "Failed to save Jito settings", variant: "destructive" });
    } finally {
      setIsSaving(false);
    }
  }

  return (
    <Card>
      <CardHeader className="pb-3">
        <CardTitle className="text-base flex items-center gap-2">
          <Zap className="w-4 h-4 text-primary" />
          Jito MEV Settings
        </CardTitle>
      </CardHeader>
      <CardContent className="space-y-4">
        {savedOk && (
          <div className="flex items-start gap-2 rounded-lg bg-green-500/10 border border-green-500/20 px-3 py-2.5 text-xs text-green-400">
            <CheckCircle className="w-3.5 h-3.5 mt-0.5 flex-shrink-0" />
            <span>
              Settings saved. Changes apply to the <strong>next order</strong> — no engine restart required.
            </span>
          </div>
        )}

        <div className="space-y-1.5">
          <Label htmlFor="jito-tip-pct" className="text-xs">
            Tip Percent
            <span className="ml-1.5 text-muted-foreground font-normal">
              (JITO_TIP_PERCENT — fraction of trade value, e.g. 0.001 = 0.1%)
            </span>
          </Label>
          <div className="flex items-center gap-2">
            <Input
              id="jito-tip-pct"
              value={tipPercent}
              onChange={(e) => { setTipPercent(e.target.value); setSavedOk(false); }}
              placeholder="0.001"
              className="font-mono text-xs max-w-[160px]"
            />
            <span className="text-xs text-muted-foreground">
              = {tipPercentDisplay(tipPercent)} of trade value
            </span>
          </div>
        </div>

        <div className="grid grid-cols-1 sm:grid-cols-2 gap-4">
          <div className="space-y-1.5">
            <Label htmlFor="jito-floor" className="text-xs">
              Tip Floor
              <span className="ml-1.5 text-muted-foreground font-normal">(JITO_TIP_FLOOR_LAMPORTS)</span>
            </Label>
            <Input
              id="jito-floor"
              value={tipFloor}
              onChange={(e) => { setTipFloor(e.target.value); setSavedOk(false); }}
              placeholder="5000"
              className="font-mono text-xs"
            />
            <p className="text-[11px] text-muted-foreground">Min tip: {lamportsToSol(tipFloor)}</p>
          </div>

          <div className="space-y-1.5">
            <Label htmlFor="jito-ceiling" className="text-xs">
              Tip Ceiling
              <span className="ml-1.5 text-muted-foreground font-normal">(JITO_TIP_CEILING_LAMPORTS)</span>
            </Label>
            <Input
              id="jito-ceiling"
              value={tipCeiling}
              onChange={(e) => { setTipCeiling(e.target.value); setSavedOk(false); }}
              placeholder="10000000"
              className="font-mono text-xs"
            />
            <p className="text-[11px] text-muted-foreground">Max tip: {lamportsToSol(tipCeiling)}</p>
          </div>
        </div>

        <div className="flex items-center gap-3">
          <Switch
            id="jito-sim"
            checked={simEnabled}
            onCheckedChange={(v) => { setSimEnabled(v); setSavedOk(false); }}
          />
          <Label htmlFor="jito-sim" className="text-xs cursor-pointer">
            Pre-submission Simulation
            <span className="ml-1.5 text-muted-foreground font-normal">
              (JITO_SIMULATION_ENABLED)
            </span>
          </Label>
          <Badge
            variant="secondary"
            className={cn(
              "text-xs",
              simEnabled ? "bg-green-500/15 text-green-400" : "bg-muted text-muted-foreground"
            )}
          >
            {simEnabled ? "Enabled" : "Disabled"}
          </Badge>
        </div>

        <Button onClick={handleSave} disabled={isSaving} size="sm">
          {isSaving ? (
            <Loader2 className="w-3.5 h-3.5 animate-spin mr-1.5" />
          ) : (
            <Save className="w-3.5 h-3.5 mr-1.5" />
          )}
          Save Jito settings
        </Button>
      </CardContent>
    </Card>
  );
}

// ── Safe Wallet Setup Guide ───────────────────────────────────────────────────

function Code({ children }: { children: React.ReactNode }) {
  return (
    <code className="font-mono bg-secondary/60 rounded px-1 py-0.5 text-[11px] text-foreground/90 break-all">
      {children}
    </code>
  );
}

function WalletSetupGuide() {
  return (
    <div className="mt-3 rounded-lg border border-dashed border-border/60 px-4 py-4 space-y-4 text-xs text-foreground/80">
      <p className="font-semibold text-sm text-foreground/90 flex items-center gap-2">
        <Shield className="w-4 h-4 text-primary" />
        How to set up your wallet (safe method)
      </p>

      <div className="rounded-lg bg-yellow-500/10 border border-yellow-500/20 px-3 py-2 text-yellow-400 text-xs">
        <strong>Never paste your private key or key bytes into any web interface.</strong>{" "}
        Use the file-path method below to keep your key material server-side only.
      </div>

      <div className="space-y-3">
        <div className="space-y-1.5">
          <div className="flex items-start gap-2">
            <span className="flex-shrink-0 w-5 h-5 rounded-full bg-primary/15 text-primary flex items-center justify-center text-[10px] font-bold">1</span>
            <p className="font-semibold text-foreground/90">Generate a keypair file on the server</p>
          </div>
          <div className="ml-7 rounded bg-muted/50 px-3 py-2 font-mono text-[11px] text-foreground/80">
            solana-keygen new --outfile /secrets/wallet.json --no-bip39-passphrase
          </div>
          <p className="ml-7 text-muted-foreground">
            Run this on the machine where the trading engine runs, not on your local computer.
          </p>
        </div>

        <div className="space-y-1.5">
          <div className="flex items-start gap-2">
            <span className="flex-shrink-0 w-5 h-5 rounded-full bg-primary/15 text-primary flex items-center justify-center text-[10px] font-bold">2</span>
            <p className="font-semibold text-foreground/90">Set the path as a Replit Secret</p>
          </div>
          <ol className="ml-7 text-muted-foreground space-y-1 list-decimal list-inside">
            <li>Open the <strong className="text-foreground/80">Secrets</strong> panel (padlock icon in the sidebar)</li>
            <li>Add a secret with key <Code>KEYPAIR_PATH</Code></li>
            <li>Set the value to the <em>file path</em>, e.g. <Code>/secrets/wallet.json</Code></li>
          </ol>
          <p className="ml-7 text-muted-foreground">
            The trading engine reads the file at startup. The key never leaves the server.
          </p>
        </div>

        <div className="space-y-1.5">
          <div className="flex items-start gap-2">
            <span className="flex-shrink-0 w-5 h-5 rounded-full bg-primary/15 text-primary flex items-center justify-center text-[10px] font-bold">3</span>
            <p className="font-semibold text-foreground/90">Alternative: register via the Wallets page</p>
          </div>
          <p className="ml-7 text-muted-foreground">
            Your operator can also register wallets via <Code>POST /api/wallets</Code> with the
            admin API key. The path is stored server-side and never returned to the UI.
          </p>
        </div>

        <div className="space-y-1.5">
          <div className="flex items-start gap-2">
            <span className="flex-shrink-0 w-5 h-5 rounded-full bg-primary/15 text-primary flex items-center justify-center text-[10px] font-bold">4</span>
            <p className="font-semibold text-foreground/90">Restart the Trading Engine</p>
          </div>
          <p className="ml-7 text-muted-foreground">
            Click <strong className="text-foreground/80">Restart</strong> next to{" "}
            <Code>rust-engine: Trading Engine</Code> in the Replit toolbar.
            The wallet badge above will turn green.
          </p>
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
          Connection and service configuration. Wallet keys are managed server-side via{" "}
          <code className="text-xs font-mono">KEYPAIR_PATH</code>.
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
                Public key is available once the Rust engine starts with the configured keypair path.
              </div>
            ) : (
              <>
                <div className="rounded-lg bg-destructive/10 px-3 py-2 text-xs text-destructive">
                  No execution wallet registered. Ask your operator to add a wallet via the
                  admin CLI or register one via the{" "}
                  <strong>Wallets page</strong>.
                </div>
                <WalletSetupGuide />
              </>
            )}
          </CardContent>
        </Card>
      )}

      {/* Strategy Preset — always shown */}
      <StrategyPresetCard />

      {/* Editable sections — populated once config loads */}
      {config && !configLoading && (
        <>
          <ConnectionCard config={config} />
          <JitoMevSettingsCard config={config} />
          <ServiceUrlsCard config={config} />
        </>
      )}

      {/* RPC status */}
      {status?.rpc && (
        <Card>
          <CardHeader className="pb-3">
            <CardTitle className="text-base flex items-center gap-2">
              {status.rpc.online ? (
                <Wifi className="w-4 h-4 text-green-500" />
              ) : (
                <WifiOff className="w-4 h-4 text-red-500" />
              )}
              RPC Status
            </CardTitle>
          </CardHeader>
          <CardContent className="space-y-2">
            <div className="flex items-center gap-3">
              <StatusIcon ok={status.rpc.online} />
              <span className="text-sm text-foreground">
                {status.rpc.online ? "Connected" : "Offline / Unreachable"}
              </span>
              {status.rpc.configured && <LatencyBadge ms={status.rpc.latencyMs ?? null} />}
            </div>
            {status.rpc.url && (
              <div className="font-mono text-xs text-muted-foreground truncate">
                {status.rpc.url}
              </div>
            )}
          </CardContent>
        </Card>
      )}
    </div>
  );
}
