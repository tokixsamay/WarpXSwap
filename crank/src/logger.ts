export type LogLevel = "debug" | "info" | "warn" | "error";

const LEVELS: Record<LogLevel, number> = { debug: 0, info: 1, warn: 2, error: 3 };

let currentLevel: LogLevel = "info";

export function setLogLevel(level: LogLevel): void {
  currentLevel = level;
}

function ts(): string {
  return new Date().toISOString();
}

function log(level: LogLevel, ctx: string, msg: string, extra?: unknown): void {
  if (LEVELS[level] < LEVELS[currentLevel]) return;
  const prefix = `[${ts()}] [${level.toUpperCase().padEnd(5)}] [${ctx}]`;
  const line   = extra !== undefined
    ? `${prefix} ${msg} ${JSON.stringify(extra)}`
    : `${prefix} ${msg}`;
  if (level === "error") {
    console.error(line);
  } else if (level === "warn") {
    console.warn(line);
  } else {
    console.log(line);
  }
}

export const logger = {
  debug: (ctx: string, msg: string, extra?: unknown) => log("debug", ctx, msg, extra),
  info:  (ctx: string, msg: string, extra?: unknown) => log("info",  ctx, msg, extra),
  warn:  (ctx: string, msg: string, extra?: unknown) => log("warn",  ctx, msg, extra),
  error: (ctx: string, msg: string, extra?: unknown) => log("error", ctx, msg, extra),
};
                                                            
