import express, { type Express } from "express";
import cors from "cors";
import pinoHttp from "pino-http";
import path from "path";
import { fileURLToPath } from "url";
import router from "./routes";
import { logger } from "./lib/logger";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

const app: Express = express();

app.use(
  pinoHttp({
    logger,
    serializers: {
      req(req) {
        return {
          id: req.id,
          method: req.method,
          url: req.url?.split("?")[0],
        };
      },
      res(res) {
        return {
          statusCode: res.statusCode,
        };
      },
    },
  }),
);
app.use(cors());
app.use(express.json());
app.use(express.urlencoded({ extended: true }));

app.use("/api", router);

// ── Dashboard static serving (production only) ────────────────────────────────
// The dashboard React app is built into artifacts/dashboard/dist/public during
// the deployment build step and served directly from Express so that only one
// port (8080) is needed — a requirement for Autoscale deployments.
if (process.env["NODE_ENV"] === "production") {
  // Resolve relative to the monorepo root (two levels above dist/index.mjs)
  const dashboardDist = path.resolve(
    __dirname,
    "../../..",
    "artifacts/dashboard/dist/public",
  );

  app.use("/dashboard", express.static(dashboardDist));

  // SPA fallback — serve index.html for any /dashboard/* route so client-side
  // routing inside the React app works correctly.
  app.get(["/dashboard", "/dashboard/*"], (_req, res) => {
    res.sendFile(path.join(dashboardDist, "index.html"));
  });

  // Redirect bare root to the dashboard so the published URL is immediately useful.
  app.get("/", (_req, res) => res.redirect(301, "/dashboard/"));
}

export default app;
