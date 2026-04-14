import { useState } from "react";
import { useListTrades, useSubmitOrder } from "@workspace/api-client-react";
import { useQueryClient } from "@tanstack/react-query";
import { cn, formatSol, formatAge, shortenAddress } from "@/lib/utils";
import { ArrowUpRight, ArrowDownRight, Send } from "lucide-react";

function StatusChip({ status }: { status: string }) {
  const cfg: Record<string, string> = {
    Executed: "bg-green-400/10 text-green-400",
    Pending: "bg-amber-400/10 text-amber-400",
    Executing: "bg-blue-400/10 text-blue-400",
    Failed: "bg-red-400/10 text-red-400",
    Cancelled: "bg-muted text-muted-foreground",
  };
  return (
    <span className={cn("px-2 py-0.5 rounded text-xs font-medium", cfg[status] ?? "bg-muted text-muted-foreground")}>
      {status}
    </span>
  );
}

export default function TradesPage() {
  const qc = useQueryClient();
  const [strategy, setStrategy] = useState("");
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const { data: trades, isLoading } = useListTrades({ limit: 100 }) as { data: any[] | undefined; isLoading: boolean };

  const submitOrder = useSubmitOrder();
  const [form, setForm] = useState({
    tokenMint: "",
    side: "BUY",
    amountSol: "",
    orderType: "MARKET",
    slippageBps: 100,
  });
  const [showForm, setShowForm] = useState(false);
  const [formMsg, setFormMsg] = useState<string | null>(null);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setFormMsg(null);
    try {
      // eslint-disable-next-line @typescript-eslint/no-explicit-any
      const result: any = await submitOrder.mutateAsync({
        data: {
          tokenMint: form.tokenMint,
          side: form.side as "BUY" | "SELL",
          amountSol: parseFloat(form.amountSol),
          orderType: form.orderType as "MARKET" | "LIMIT",
          slippageBps: form.slippageBps,
          strategyName: "manual",
        },
      });
      if (result.success) {
        setFormMsg("Order submitted: " + result.orderId);
        qc.invalidateQueries({ queryKey: ["listTrades"] });
        setForm({ tokenMint: "", side: "BUY", amountSol: "", orderType: "MARKET", slippageBps: 100 });
      } else {
        setFormMsg("Error: " + result.message);
      }
    } catch (e: unknown) {
      setFormMsg("Error: " + (e instanceof Error ? e.message : "unknown"));
    }
  };

  const filteredTrades = strategy
    ? (trades ?? []).filter((t) => t.strategy === strategy)
    : (trades ?? []);

  return (
    <div className="space-y-4 sm:space-y-6">
      {/* Page header */}
      <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-3">
        <div>
          <h1 className="text-lg font-bold">Trade History</h1>
          <p className="text-sm text-muted-foreground mt-0.5">All orders executed by the bot.</p>
        </div>
        <button
          onClick={() => setShowForm(!showForm)}
          className="flex items-center justify-center gap-2 px-4 py-3 sm:py-2.5 bg-primary text-primary-foreground rounded-lg text-sm font-semibold hover:opacity-90 transition-opacity min-h-[44px] w-full sm:w-auto"
        >
          <Send className="w-4 h-4" />
          Manual Order
        </button>
      </div>

      {/* Manual order form */}
      {showForm && (
        <form onSubmit={handleSubmit} className="bg-card border border-border rounded-xl p-4 sm:p-5 space-y-4">
          <h2 className="text-sm font-semibold">Submit Manual Order</h2>
          <div className="grid grid-cols-1 sm:grid-cols-2 gap-4">
            <div className="sm:col-span-2">
              <label className="text-xs text-muted-foreground block mb-1.5">Token Mint Address</label>
              <input
                type="text"
                required
                className="w-full bg-secondary border border-border rounded-lg px-3 py-3 sm:py-2 text-sm font-mono focus:outline-none focus:ring-1 focus:ring-primary min-h-[44px]"
                placeholder="e.g. 7xKXtg2CW87d97TXJSDpbD5jBkheTqA83TZRuJosgAsU"
                value={form.tokenMint}
                onChange={(e) => setForm({ ...form, tokenMint: e.target.value })}
              />
            </div>
            <div>
              <label className="text-xs text-muted-foreground block mb-1.5">Side</label>
              <select
                className="w-full bg-secondary border border-border rounded-lg px-3 py-3 sm:py-2 text-sm focus:outline-none focus:ring-1 focus:ring-primary min-h-[44px]"
                value={form.side}
                onChange={(e) => setForm({ ...form, side: e.target.value })}
              >
                <option value="BUY">BUY</option>
                <option value="SELL">SELL</option>
              </select>
            </div>
            <div>
              <label className="text-xs text-muted-foreground block mb-1.5">Amount (SOL)</label>
              <input
                type="number"
                required
                step="0.001"
                min="0.001"
                className="w-full bg-secondary border border-border rounded-lg px-3 py-3 sm:py-2 text-sm tabular-nums focus:outline-none focus:ring-1 focus:ring-primary min-h-[44px]"
                placeholder="0.05"
                value={form.amountSol}
                onChange={(e) => setForm({ ...form, amountSol: e.target.value })}
              />
            </div>
            <div>
              <label className="text-xs text-muted-foreground block mb-1.5">Order Type</label>
              <select
                className="w-full bg-secondary border border-border rounded-lg px-3 py-3 sm:py-2 text-sm focus:outline-none focus:ring-1 focus:ring-primary min-h-[44px]"
                value={form.orderType}
                onChange={(e) => setForm({ ...form, orderType: e.target.value })}
              >
                <option value="MARKET">MARKET</option>
                <option value="LIMIT">LIMIT</option>
              </select>
            </div>
            <div>
              <label className="text-xs text-muted-foreground block mb-1.5">Slippage (bps)</label>
              <input
                type="number"
                min="1"
                max="1000"
                className="w-full bg-secondary border border-border rounded-lg px-3 py-3 sm:py-2 text-sm tabular-nums focus:outline-none focus:ring-1 focus:ring-primary min-h-[44px]"
                value={form.slippageBps}
                onChange={(e) => setForm({ ...form, slippageBps: parseInt(e.target.value) })}
              />
            </div>
          </div>
          <div className="flex flex-col sm:flex-row sm:items-center gap-3">
            <button
              type="submit"
              disabled={submitOrder.isPending}
              className="flex items-center justify-center px-4 py-3 sm:py-2 bg-primary text-primary-foreground rounded-lg text-sm font-semibold hover:opacity-90 disabled:opacity-50 min-h-[44px] w-full sm:w-auto"
            >
              {submitOrder.isPending ? "Submitting..." : "Submit Order"}
            </button>
            {formMsg && (
              <span className={cn("text-xs text-center sm:text-left", formMsg.startsWith("Error") ? "text-red-400" : "text-green-400")}>
                {formMsg}
              </span>
            )}
          </div>
        </form>
      )}

      {/* Filters */}
      <div className="flex items-center gap-3">
        <select
          className="bg-secondary border border-border rounded-lg px-3 py-2.5 sm:py-1.5 text-sm sm:text-xs focus:outline-none focus:ring-1 focus:ring-primary min-h-[44px] sm:min-h-0"
          value={strategy}
          onChange={(e) => setStrategy(e.target.value)}
        >
          <option value="">All Strategies</option>
          <option value="sniper">Sniper</option>
          <option value="momentum">Momentum</option>
          <option value="manual">Manual</option>
        </select>
        <span className="text-xs text-muted-foreground">
          {filteredTrades.length} records
        </span>
      </div>

      {/* Trades table — horizontally scrollable on mobile */}
      <div className="bg-card border border-border rounded-lg overflow-hidden overflow-x-auto">
        <table className="w-full text-xs min-w-[640px]">
          <thead>
            <tr className="text-muted-foreground border-b border-border bg-secondary/20">
              <th className="text-left py-3 px-4 font-medium">Token</th>
              <th className="text-left py-3 px-4 font-medium">Side</th>
              <th className="text-right py-3 px-4 font-medium">Amount</th>
              <th className="text-right py-3 px-4 font-medium">PnL</th>
              <th className="text-left py-3 px-4 font-medium">Strategy</th>
              <th className="text-left py-3 px-4 font-medium">Status</th>
              <th className="text-left py-3 px-4 font-medium">Signature</th>
              <th className="text-right py-3 px-4 font-medium">Time</th>
            </tr>
          </thead>
          <tbody>
            {isLoading ? (
              <tr><td colSpan={8} className="py-8 text-center text-muted-foreground">Loading...</td></tr>
            ) : filteredTrades.length > 0 ? (
              filteredTrades.map((t) => (
                <tr key={t.id} className="border-b border-border/50 hover:bg-secondary/20 transition-colors">
                  <td className="py-3 px-4">
                    <span className="font-medium">{t.tokenSymbol || shortenAddress(t.mint, 4)}</span>
                  </td>
                  <td className="py-3 px-4">
                    <span className={cn("px-1.5 py-0.5 rounded text-xs font-semibold",
                      t.side === "BUY" ? "bg-green-400/10 text-green-400" : "bg-red-400/10 text-red-400"
                    )}>
                      {t.side === "BUY" ? <ArrowUpRight className="inline w-3 h-3" /> : <ArrowDownRight className="inline w-3 h-3" />}
                      {t.side}
                    </span>
                  </td>
                  <td className="py-3 px-4 text-right tabular-nums">{formatSol(t.amountSol)}</td>
                  <td className="py-3 px-4 text-right tabular-nums">
                    {t.pnlSol != null ? (
                      <span className={t.pnlSol >= 0 ? "text-green-400" : "text-red-400"}>
                        {t.pnlSol >= 0 ? "+" : ""}{t.pnlSol.toFixed(4)}
                      </span>
                    ) : "—"}
                  </td>
                  <td className="py-3 px-4">
                    <span className="px-2 py-0.5 rounded bg-secondary text-muted-foreground">{t.strategy}</span>
                  </td>
                  <td className="py-3 px-4"><StatusChip status={t.status} /></td>
                  <td className="py-3 px-4 font-mono">
                    {t.signature ? (
                      <a
                        href={`https://solscan.io/tx/${t.signature}`}
                        target="_blank"
                        rel="noopener noreferrer"
                        className="text-primary hover:underline"
                      >
                        {t.signature.slice(0, 8)}...
                      </a>
                    ) : "—"}
                  </td>
                  <td className="py-3 px-4 text-right text-muted-foreground">{formatAge(t.createdAt)}</td>
                </tr>
              ))
            ) : (
              <tr><td colSpan={8} className="py-8 text-center text-muted-foreground">No trades found</td></tr>
            )}
          </tbody>
        </table>
      </div>
    </div>
  );
}
