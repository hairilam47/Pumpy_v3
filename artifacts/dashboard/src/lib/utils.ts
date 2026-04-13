import { clsx, type ClassValue } from "clsx"
import { twMerge } from "tailwind-merge"

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs))
}

export function formatSol(amount: number, decimals = 4): string {
  if (Math.abs(amount) < 0.0001 && amount !== 0) {
    return amount.toExponential(2) + " SOL";
  }
  return amount.toFixed(decimals) + " SOL";
}

export function formatPnl(amount: number): string {
  const sign = amount >= 0 ? "+" : "";
  return sign + formatSol(amount);
}

export function formatPercent(value: number, decimals = 1): string {
  return value.toFixed(decimals) + "%";
}

export function shortenAddress(addr: string, chars = 4): string {
  if (!addr || addr.length < chars * 2 + 3) return addr || "";
  return `${addr.slice(0, chars)}...${addr.slice(-chars)}`;
}

export function formatDate(dateStr: string | undefined): string {
  if (!dateStr) return "—";
  return new Date(dateStr).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
}

export function formatAge(dateStr: string | undefined): string {
  if (!dateStr) return "—";
  const diff = Date.now() - new Date(dateStr).getTime();
  const secs = Math.floor(diff / 1000);
  if (secs < 60) return `${secs}s ago`;
  const mins = Math.floor(secs / 60);
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.floor(mins / 60);
  return `${hours}h ago`;
}
