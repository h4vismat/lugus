// Display/default calculations only. The Rust engine validates all submitted trades.
const SCALE=10n**18n;
function coefficient(value:string):bigint{
 if(!/^-?\d+(\.\d{1,18})?$/.test(value)||value.length>64)throw new Error('Enter a plain decimal with at most 18 fractional digits.');
 const negative=value.startsWith('-');const [whole,fraction='']=value.replace(/^-/,'').split('.');const n=BigInt(whole)*SCALE+BigInt(fraction.padEnd(18,'0'));return negative?-n:n;
}
function rounded(n:bigint,d:bigint):bigint{const sign=n<0n?-1n:1n;const a=n<0n?-n:n;let q=a/d;const r=a%d;if(r*2n>d||(r*2n===d&&q%2n!==0n))q++;return q*sign;}
function centsText(c:bigint):string{const sign=c<0n?'-':'';const s=(c<0n?-c:c).toString().padStart(3,'0');return `${sign}${s.slice(0,-2)}.${s.slice(-2)}`;}
export function formatUsd(value:string|null):string{if(value===null)return 'Unpriced';const s=centsText(rounded(coefficient(value),10n**16n));const [whole,fraction]=s.replace(/^-/,'').split('.');return `${s.startsWith('-')?'-':''}$${whole.replace(/\B(?=(\d{3})+(?!\d))/g,',')}.${fraction}`;}
export function decimalProduct(a:string,b:string):string{return centsText(rounded(coefficient(a)*coefficient(b),10n**34n));}
export function overviewLabels(v:{valuation:{complete:boolean;total_value:string|null;priced_subtotal:string}}){return {valueLabel:v.valuation.complete?'Total value':'Priced subtotal',value:formatUsd(v.valuation.complete?v.valuation.total_value:v.valuation.priced_subtotal),allocationAvailable:v.valuation.complete};}
