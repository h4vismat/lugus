import type { Rpc } from "../portfolio/api.ts";
export const comparisonApi =
  (rpc: Rpc) =>
  <T>(command: object) =>
    rpc<T>({ operation: "comparison", command });
