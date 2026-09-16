import type { Rpc } from "../portfolio/api";
export const companyApi =
  (rpc: Rpc) =>
  <T>(command: object) =>
    rpc<T>({ operation: "company", command });
