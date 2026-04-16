import * as grpc from "@grpc/grpc-js";
import * as protoLoader from "@grpc/proto-loader";
import path from "path";

const PROTO_PATH = process.env.PROTO_PATH
  ?? path.resolve(process.cwd(), "..", "..", "rust-engine", "proto", "bot.proto");

const GRPC_HOST = process.env.GRPC_HOST ?? "localhost";
const GRPC_PORT = process.env.GRPC_PORT ?? "50051";
const GRPC_ADDR = `${GRPC_HOST}:${GRPC_PORT}`;

let _client: grpc.Client | null = null;

function getClient(): grpc.Client {
  if (!_client) {
    const def = protoLoader.loadSync(PROTO_PATH, {
      keepCase: true,
      longs: String,
      enums: String,
      defaults: true,
      oneofs: true,
    });
    const pkg = grpc.loadPackageDefinition(def) as Record<string, grpc.GrpcObject>;
    const botPkg = pkg["bot"] as Record<string, grpc.ServiceClientConstructor>;
    const BotService = botPkg["Bot"];
    _client = new BotService(GRPC_ADDR, grpc.credentials.createInsecure(), {
      "grpc.keepalive_time_ms": 10_000,
      "grpc.keepalive_timeout_ms": 5_000,
      "grpc.keepalive_permit_without_calls": 1,
    });
  }
  return _client;
}

type RawClient = Record<string, unknown>;

function callUnary<T>(method: string, request: Record<string, unknown>): Promise<T> {
  return new Promise((resolve, reject) => {
    const client = getClient() as unknown as RawClient;
    const fn = client[method];
    if (typeof fn !== "function") {
      reject(new Error(`gRPC method not found: ${method}`));
      return;
    }
    (fn as (
      req: Record<string, unknown>,
      cb: (err: grpc.ServiceError | null, res: T) => void
    ) => void).call(client, request, (err, res) => {
      if (err) reject(err);
      else resolve(res);
    });
  });
}

export interface SubmitOrderRequest {
  token_mint: string;
  order_type: string;
  side: string;
  amount: number;
  slippage_bps: number;
  strategy_name: string;
  max_sol_cost?: number;
  min_sol_output?: number;
  metadata?: Record<string, string>;
  client_order_id?: string;
  trace_id?: string;
}

export interface SubmitOrderResponse {
  order_id: string;
  success: boolean;
  message: string;
}

export interface OrderStatusResponse {
  order_id: string;
  status: string;
  signature: string;
  error: string;
  executed_at?: string;
}

export interface TokenInfoResponse {
  mint: string;
  name: string;
  symbol: string;
  price: number;
  liquidity_sol: number;
  market_cap_sol: number;
  volume_24h_sol: number;
  holder_count: number;
  bonding_curve_progress: number;
}

export interface PortfolioSummaryResponse {
  total_value_sol: number;
  cash_balance_sol: number;
  positions_value_sol: number;
  daily_pnl_sol: number;
  total_pnl_sol: number;
  open_positions_count: number;
  win_rate: number;
}

export interface OrderUpdate {
  order_id: string;
  token_mint?: string;
  status: string;
  signature?: string;
  error?: string;
  executed_at?: string;
  executed_price?: number;
  executed_amount?: number;
}

export const grpcBot = {
  async submitOrder(req: SubmitOrderRequest): Promise<SubmitOrderResponse> {
    return callUnary<SubmitOrderResponse>("SubmitOrder", req as unknown as Record<string, unknown>);
  },

  async cancelOrder(orderId: string): Promise<{ success: boolean; message: string }> {
    return callUnary("CancelOrder", { order_id: orderId });
  },

  async getOrderStatus(orderId: string): Promise<OrderStatusResponse> {
    return callUnary<OrderStatusResponse>("GetOrderStatus", { order_id: orderId });
  },

  async getTokenInfo(mint: string): Promise<TokenInfoResponse> {
    return callUnary<TokenInfoResponse>("GetTokenInfo", { token_mint: mint });
  },

  async getPortfolioSummary(): Promise<PortfolioSummaryResponse> {
    return callUnary<PortfolioSummaryResponse>("GetPortfolioSummary", {});
  },

  /**
   * Open a server-streaming RPC for order updates.
   * Calls `onUpdate` for each OrderUpdate received.
   * Returns an abort function to cancel the stream.
   */
  streamOrders(
    orderIds: string[],
    onUpdate: (update: OrderUpdate) => void,
    onEnd?: (err?: grpc.ServiceError) => void
  ): () => void {
    const client = getClient() as unknown as RawClient;
    const fn = client["StreamOrders"] as (req: Record<string, unknown>) => grpc.ClientReadableStream<OrderUpdate>;
    const call = fn.call(client, { order_ids: orderIds });

    call.on("data", (update: OrderUpdate) => onUpdate(update));
    call.on("end", () => onEnd?.());
    call.on("error", (err: grpc.ServiceError) => onEnd?.(err));

    return () => call.cancel();
  },
};
