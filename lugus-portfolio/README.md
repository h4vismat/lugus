# Portfolio accounting

Pure Rust accounting for long-only stocks, ETFs and cash in USD. No provider, database or agent dependencies. The application layer supplies validated ledgers and compatible price observations.

Accounts begin with full history (zero cash and holdings) or opening cash and remaining purchase lots. Original purchase dates and tie order determine FIFO within an account. Simplified lots and assumed dates remain marked in the results. Opening balances are not deposits or historical profit.

Replay orders events by date and explicit same-day order. Purchases add gross plus fees to basis; sales consume the oldest lots and deduct fees from proceeds. Deposits, withdrawals, dividends and standalone fees affect cash separately. Every intermediate state must have nonnegative cash and holdings. Splits change quantities without changing total basis; unrepresentable ratios fail.

Decimals serialize as strings, use 18 fractional digits and a bounded 38-digit coefficient. Money inputs must resolve to cents. Allocation uses half-even rounding with the final lot receiving the remainder, conserving basis and proceeds. No binary floating point enters accounting.

Valuation returns a priced subtotal and explicit unpriced holdings. Total value, total unrealized P&L and allocation percentages are absent until all holdings have compatible USD prices. Price dates and observation IDs remain attached to values. Realized P&L is independent of price refresh; dividends and standalone fees are separate figures.

```sh
cargo test -p lugus-portfolio
```

Tests cover worked FIFO examples, partial-disposal basis conservation, equivalent opening lots, splits, date/order ambiguity, intermediate negative cash, overselling, decimal bounds and incomplete valuations. This is portfolio bookkeeping, not tax reporting or a historical performance engine.
