import type { Header, View } from "./types.ts";

export interface PortfolioLoadState {
  scope: string;
  status: "loading" | "ready" | "error";
  headers: Header[];
  whole: View | null;
  view: View | null;
  error: string;
}

export function portfolioScope(
  portfolioId: string | null,
  accountId: string | null,
) {
  return JSON.stringify([portfolioId, accountId]);
}

export function beginPortfolioLoad(scope: string): PortfolioLoadState {
  return {
    scope,
    status: "loading",
    headers: [],
    whole: null,
    view: null,
    error: "",
  };
}

/** Gate rendering immediately, before the next selection's effect has started. */
export function selectedPortfolioLoad(
  state: PortfolioLoadState,
  scope: string,
): PortfolioLoadState {
  return state.scope === scope ? state : beginPortfolioLoad(scope);
}

export function finishPortfolioLoad(
  state: PortfolioLoadState,
  scope: string,
  headers: Header[],
  whole: View | null,
  view: View | null,
): PortfolioLoadState {
  return state.scope === scope
    ? { scope, status: "ready", headers, whole, view, error: "" }
    : state;
}

export function failPortfolioLoad(
  state: PortfolioLoadState,
  scope: string,
  error: string,
): PortfolioLoadState {
  return state.scope === scope
    ? { ...beginPortfolioLoad(scope), status: "error", error }
    : state;
}

/** A refresh may only replace the revision that was displayed when it began. */
export function finishPortfolioRefresh(
  state: PortfolioLoadState,
  scope: string,
  expectedRevision: string,
  whole: View,
  view: View,
): PortfolioLoadState {
  if (
    state.status !== "ready" ||
    state.scope !== scope ||
    state.view?.revision !== expectedRevision
  )
    return state;
  return finishPortfolioLoad(state, scope, state.headers, whole, view);
}
