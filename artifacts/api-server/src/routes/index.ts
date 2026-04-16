import { Router, type IRouter } from "express";
import healthRouter from "./health";
import botRouter from "./bot";
import tradesRouter from "./trades";
import settingsRouter from "./settings";
import walletsRouter from "./wallets";
import adminRouter from "./admin";
import tokenMetricsRouter from "./token-metrics";

const router: IRouter = Router();

router.use(healthRouter);
router.use(tradesRouter);
router.use(botRouter);
router.use(settingsRouter);
router.use(walletsRouter);
router.use(adminRouter);
router.use(tokenMetricsRouter);

export default router;
