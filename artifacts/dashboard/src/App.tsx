import { Switch, Route, Router as WouterRouter, Link, useLocation } from "wouter";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { Toaster } from "@/components/ui/toaster";
import { TooltipProvider } from "@/components/ui/tooltip";
import NotFound from "@/pages/not-found";
import DashboardPage from "@/pages/Dashboard";
import StrategiesPage from "@/pages/Strategies";
import TokensPage from "@/pages/Tokens";
import TradesPage from "@/pages/Trades";
import SettingsPage from "@/pages/Settings";
import { cn } from "@/lib/utils";
import {
  LayoutDashboard, Bot, Coins, History, Settings2,
  Activity, Github, ExternalLink
} from "lucide-react";

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      retry: 1,
      staleTime: 3000,
    },
  },
});

const NAV_LINKS = [
  { href: "/", label: "Dashboard", icon: LayoutDashboard },
  { href: "/strategies", label: "Strategies", icon: Bot },
  { href: "/tokens", label: "Tokens", icon: Coins },
  { href: "/trades", label: "Trades", icon: History },
  { href: "/settings", label: "Settings", icon: Settings2 },
];

function Sidebar() {
  const [location] = useLocation();
  return (
    <aside className="hidden md:flex w-56 flex-shrink-0 bg-sidebar border-r border-sidebar-border flex-col min-h-screen">
      {/* Logo */}
      <div className="px-5 py-5 border-b border-sidebar-border">
        <div className="flex items-center gap-2.5">
          <div className="w-7 h-7 rounded-lg bg-primary flex items-center justify-center">
            <Activity className="w-4 h-4 text-primary-foreground" />
          </div>
          <div>
            <div className="text-sm font-bold text-sidebar-foreground leading-none">PumpyPumpy</div>
            <div className="text-xs text-muted-foreground mt-0.5">Trading Bot</div>
          </div>
        </div>
      </div>

      {/* Nav links */}
      <nav className="flex-1 p-3 space-y-1">
        {NAV_LINKS.map(({ href, label, icon: Icon }) => {
          const isActive = href === "/" ? location === "/" : location.startsWith(href);
          return (
            <Link
              key={href}
              href={href}
              className={cn(
                "flex items-center gap-3 px-3 py-2.5 rounded-lg text-sm transition-colors",
                isActive
                  ? "bg-primary/10 text-primary font-medium"
                  : "text-sidebar-foreground hover:bg-sidebar-accent hover:text-sidebar-accent-foreground"
              )}
            >
              <Icon className="w-4 h-4 flex-shrink-0" />
              {label}
            </Link>
          );
        })}
      </nav>

      {/* Footer */}
      <div className="p-3 border-t border-sidebar-border space-y-1">
        <a
          href="https://github.com/hairilam47/PumpyPumpyFunBotTrade"
          target="_blank"
          rel="noopener noreferrer"
          className="flex items-center gap-2 px-3 py-2 rounded-lg text-xs text-muted-foreground hover:text-sidebar-foreground hover:bg-sidebar-accent transition-colors"
        >
          <Github className="w-3.5 h-3.5" />
          GitHub Repo
          <ExternalLink className="w-3 h-3 ml-auto" />
        </a>
        <div className="px-3 py-1">
          <div className="text-xs text-muted-foreground">Pump.fun Program</div>
          <div className="text-xs font-mono text-muted-foreground/70 truncate">6EF8rre...F6P</div>
        </div>
      </div>
    </aside>
  );
}

function MobileHeader() {
  return (
    <header className="md:hidden sticky top-0 z-40 flex items-center gap-2.5 px-4 h-12 bg-sidebar border-b border-sidebar-border flex-shrink-0">
      <div className="w-7 h-7 rounded-lg bg-primary flex items-center justify-center">
        <Activity className="w-4 h-4 text-primary-foreground" />
      </div>
      <div className="leading-none">
        <div className="text-sm font-bold text-sidebar-foreground leading-none">PumpyPumpy</div>
        <div className="text-[10px] text-muted-foreground mt-0.5">Trading Bot</div>
      </div>
    </header>
  );
}

function BottomNav() {
  const [location] = useLocation();
  return (
    <nav className="md:hidden fixed bottom-0 left-0 right-0 z-50 flex items-stretch bg-sidebar border-t border-sidebar-border bottom-nav-safe">
      {NAV_LINKS.map(({ href, label, icon: Icon }) => {
        const isActive = href === "/" ? location === "/" : location.startsWith(href);
        return (
          <Link
            key={href}
            href={href}
            className={cn(
              "flex-1 flex flex-col items-center justify-center gap-1 py-2 min-h-[56px] transition-colors active:bg-sidebar-accent",
              isActive ? "text-primary" : "text-muted-foreground"
            )}
          >
            <Icon className={cn("w-5 h-5", isActive && "drop-shadow-[0_0_4px_rgba(34,197,94,0.5)]")} />
            <span className="text-[10px] leading-none font-medium">{label}</span>
          </Link>
        );
      })}
    </nav>
  );
}

function Layout({ children }: { children: React.ReactNode }) {
  return (
    <div className="flex min-h-screen bg-background">
      <Sidebar />
      <main className="flex-1 overflow-auto flex flex-col min-w-0">
        <MobileHeader />
        <div className="max-w-6xl mx-auto w-full p-3 sm:p-6 pb-24 md:pb-6">
          {children}
        </div>
      </main>
      <BottomNav />
    </div>
  );
}

function Router() {
  return (
    <Layout>
      <Switch>
        <Route path="/" component={DashboardPage} />
        <Route path="/strategies" component={StrategiesPage} />
        <Route path="/tokens" component={TokensPage} />
        <Route path="/trades" component={TradesPage} />
        <Route path="/settings" component={SettingsPage} />
        <Route component={NotFound} />
      </Switch>
    </Layout>
  );
}

export default function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <TooltipProvider>
        <WouterRouter base={import.meta.env.BASE_URL.replace(/\/$/, "")}>
          <Router />
        </WouterRouter>
        <Toaster />
      </TooltipProvider>
    </QueryClientProvider>
  );
}
