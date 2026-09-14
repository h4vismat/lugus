export interface Selection{portfolioId:string|null;accountId:string|null;generation:number}
export function acceptsPortfolioResult(current:Selection,origin:Selection){return current.portfolioId===origin.portfolioId&&current.accountId===origin.accountId&&current.generation===origin.generation;}
