export type Decimal=string;
export interface Page<T>{items:T[];next_offset:number|null;revision:string}
export interface Header{id:string;name:string;currency:string;revision:string}
export interface Instrument{id:string;name:string;symbol:string;asset_kind:'stock'|'etf';currency:string;binding:{instance_id:string;native_id:{namespace:string;value:string}}|null}
export interface OpeningLot{id:string;instrument_id:string;acquired:string;tie_order:number;quantity:string;basis:string;simplified:boolean;date_assumed:boolean}
export type Opening={kind:'full_history'}|{kind:'existing';cash:string;lots:OpeningLot[]};
export interface Account{id:string;name:string;revision:string;start:string;opening?:Opening;simplified?:boolean}
export interface Event{id:string;date:string;order:number;kind:Record<string,unknown>&{kind:string}}
export interface TransactionRow{account_id:string;account_name:string;event:Event}
export interface Lot{id:string;instrument_id:string;acquired:string;quantity:string;basis:string;simplified:boolean}
export interface LotRow{account_id:string;account_name:string;lot:Lot}
export interface Holding{instrument_id:string;quantity:string;basis:string;market_value:string|null;unrealized:string|null;price:{close:string;date:string}|null;unpriced_reason:string|null;allocation_percent:string|null;simplified:boolean}
export interface View{benchmark_instance_id?:string|null;dashboard?:DashboardMetrics|null;id:string;name:string;revision:string;as_of:string;accounts:Account[];instruments:Instrument[];valuation:{cash:string;holdings:Holding[];priced_subtotal:string;total_value:string|null;unrealized:string|null;complete:boolean;cash_allocation_percent:string|null};realized:string;dividends:string;standalone_fees:string;trade_fees:string;deposits:string;withdrawals:string;price_status:string[]}
export interface Command{request_id:string;portfolio_id:string|null;expected_revision:string;mutation:Record<string,unknown>}
export interface Receipt{portfolio_id:string;revision:string;account_ids:string[];instrument_ids:string[]}
export interface Preview{view:View;states:{account_id:string;matches:{sale_id:string;lot_id:string;quantity:string;basis:string;net_proceeds:string;realized:string;simplified:boolean}[]}[]}
export interface Snapshot{id:string;summary:View;created_at:string}
export interface Audit{request_id:string;revision:string;recorded_at:string;mutation:Record<string,unknown>}

export interface AllocationMetric {key:string;label:string;value:Decimal;percent:Decimal}
export interface DashboardMetrics {invested_market_value:Decimal|null;by_asset_type:AllocationMetric[];largest_instrument_id:string|null;largest_weight:Decimal|null;top_two_weight:Decimal|null;holdings:{instrument_id:string;unrealized_percent:Decimal|null}[]}
export interface HistoryRange {start:string|null;end:string}
export interface ProviderIdentity {instance_id:string;plugin_id:string;plugin_version:string}
export interface HistoryKey {portfolio_id:string;revision:string;account_id:string|null;requested_range:HistoryRange;calculation_version:number;benchmark_provider:ProviderIdentity|null;bindings_fingerprint:string}
export type HistoryStatus='running'|'complete'|'partial'|'failed'|'cancelled'|'timed_out'|'stale_revision'|'interrupted'|'interrupted_or_external';
export interface HistoryIssue {code:string;date:string;instrument_id:string|null;account_id:string|null}
export interface PerformanceSummary {portfolio_return_percent:Decimal|null;benchmark_return_percent:Decimal|null;difference_pp:Decimal|null}
export interface PerformancePoint {date:string;value:Decimal|null;deposits:Decimal|null;withdrawals:Decimal;opening_contribution:Decimal|null;portfolio_growth:Decimal|null;portfolio_return_percent:Decimal|null;segment_return_percent:Decimal|null;segment:number;benchmark_return_percent:Decimal|null;hypothetical_value:Decimal|null;issues:HistoryIssue[]}
export interface PortfolioHistoryHeader {id:string;key:HistoryKey;status:HistoryStatus;baseline:string|null;effective_end:string|null;summary:PerformanceSummary;row_count:number;evidence_count:number;issues:HistoryIssue[];issue_count:number;input_fingerprint:string|null;created_at:string;finished_at:string|null;error:string|null}
export interface PortfolioHistoryPage {result_id:string;key:HistoryKey;items:PerformancePoint[];next_offset:number|null}
export interface HistoryEvidenceRef {manifest?:{instrument:{namespace:string;value:string};coverage_start:string;anchor:string;last_completed_session:string;retrieved_at:string;calendar:string;calendar_version:string;normalization_version:number;source_basis:string}|null;source_url?:string|null;instrument_id:string|null;fetch_id:string;run_id:string;provider:ProviderIdentity;manifest_fingerprint:string}

export interface HistoryEvidencePage {result_id:string;key:HistoryKey;items:HistoryEvidenceRef[];next_offset:number|null}
