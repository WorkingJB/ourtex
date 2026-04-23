// Keeps the server's session-key TTL alive.
//
// Mirrors `HeartbeatHandle` in `apps/desktop`: republishes the content
// key at roughly 1/4 of the server's 15-minute default TTL. One missed
// refresh does not lock the workspace; two in a row does.
import { api } from "./api";

const INTERVAL_MS = 4 * 60 * 1000;

export type Heartbeat = {
  stop: () => void;
};

export function startHeartbeat(
  tenantId: string,
  contentKeyWire: string
): Heartbeat {
  const timer = setInterval(() => {
    api.publishSessionKey(tenantId, contentKeyWire).catch((err) => {
      // Don't thrash on transient failures; the next tick will retry.
      console.warn("session-key refresh failed", err);
    });
  }, INTERVAL_MS);
  return {
    stop: () => clearInterval(timer),
  };
}
