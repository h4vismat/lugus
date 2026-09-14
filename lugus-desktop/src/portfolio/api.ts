export type Rpc=<T>(request:object)=>Promise<T>;
export function createPortfolioApi(rpc:Rpc){return <T>(command:object)=>rpc<T>({operation:'portfolio',command});}
export type PortfolioApi=ReturnType<typeof createPortfolioApi>;
