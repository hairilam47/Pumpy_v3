import * as grpc from "@grpc/grpc-js";
import * as protoLoader from "@grpc/proto-loader";
import path from "path";
import { fileURLToPath } from "url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const PROTO_PATH = path.join(__dirname, "..", "bot.proto");

const GRPC_HOST = process.env.GRPC_HOST ?? "localhost";
const GRPC_PORT = process.env.GRPC_PORT ?? "50051";
const GRPC_ADDR = `${GRPC_HOST}:${GRPC_PORT}`;

let _packageDef: protoLoader.PackageDefinition | null = null;
let _grpcClient: grpc.Client | null = null;

function getPackageDef(): protoLoader.PackageDefinition {
  if (!_packageDef) {
    _packageDef = protoLoader.loadSync(PROTO_PATH, {
      keepCase: true,
      longs: String,
      enums: String,
      defaults: true,
      oneofs: true,
    });
  }
  return _packageDef;
}

function getBotService(): grpc.GrpcObject {
  const def = getPackageDef();
  return (grpc.loadPackageDefinition(def) as Record<string, grpc.GrpcObject>)["bot"] as grpc.GrpcObject;
}

function getRawClient(): grpc.Client {
  if (!_grpcClient) {
    const botService = getBotService();
    const BotService = botService["Bot"] as grpc.ServiceClientConstructor;
    _grpcClient = new BotService(
      GRPC_ADDR,
      grpc.credentials.createInsecure(),
      {
        "grpc.keepalive_time_ms": 10_000,
        "grpc.keepalive_timeout_ms": 5_000,
        "grpc.keepalive_permit_without_calls": 1,
      }
    );
  }
  return _grpcClient;
}

type GrpcMethod = (
  request: Record<string, unknown>,
  callback: (err: grpc.ServiceError | null, response: unknown) => void
) => grpc.ClientUnaryCall;

function callRpc<T>(method: string, request: Record<string, unknown>): Promise<T> {
  return new Promise((resolve, reject) => {
    const client = getRawClient() as Record<string, GrpcMethod>;
    const fn = client[method];
    if (typeof fn !== "function") {
      reject(new Error(`gRPC method not found: ${method}`));
      return;
    }
    fn.call(client, request, (err, response) => {
      if (err) reject(err);
      else resolve(response as T);
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
  status: string;
  signature?: string;
  error?: string;
  executed_at?: string;
  executed_price?: number;
  executed_amount?: number;
}

export const grpcBot = {
  async submitOrder(req: SubmitOrderRequest): Promise<SubmitOrderResponse> {
    return callRpc<SubmitOrderResponse>("SubmitOrder", req as unknown as Record<string, unknown>);
  },

  async cancelOrder(orderId: string): Promise<{ success: boolean; message: string }> {
    return callRpc("CancelOrder", { order_id: orderId });
  },

  async getOrderStatus(orderId: string): Promise<OrderStatusResponse> {
    return callRpc<OrderStatusResponse>("GetOrderStatus", { order_id: orderId });
  },

  async getPortfolioSummary(): Promise<PortfolioSummaryResponse> {
    return callRpc<PortfolioSummaryResponse>("GetPortfolioSummary", {});
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
    const client = getRawClient() as Record<string, (req: Record<string, unknown>) => grpc.ClientReadableStream<OrderUpdate>>;
    const call = client["StreamOrders"]!({ order_ids: orderIds });

    call.on("data", (update: OrderUpdate) => {
      onUpdate(update);
    });

    call.on("end", () => {
      onEnd?.();
    });

    call.on("error", (err: grpc.ServiceError) => {
      onEnd?.(err);
    });

    return () => call.cancel();
  },
};
