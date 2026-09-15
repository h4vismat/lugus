import type {PortfolioApi} from './api';
import type {View,PerformanceSummary,PortfolioHistoryHeader,PortfolioHistoryPage,PerformancePoint,HistoryEvidencePage} from './types';
import {historyRange,acceptsHistory,type Period,type HistorySelection} from './history-state';
import {mountPerformance} from './performance';
import {errorText} from './forms';
export function createHistoryController(api:PortfolioApi,root:HTMLElement,updateSummary:(summary:PerformanceSummary|null,label:string,stale:boolean)=>void,initial:Period='YTD'){
 let starting:Promise<PortfolioHistoryHeader>|null=null;
 let current:HistorySelection|null=null,view:View|null=null,online=true,alive=true,period:Period=initial,owned:{portfolioId:string;id:string}|null=null,displayed:PortfolioHistoryHeader|null=null;
 const active=(origin:HistorySelection)=>alive&&current!==null&&acceptsHistory(current,origin);
 const presentation=mountPerformance(root,{async onEvidencePage(offset){if(!displayed)throw new Error('No saved history selected.');const header=displayed;const page=await api<HistoryEvidencePage>({kind:'history_evidence',portfolio_id:header.key.portfolio_id,id:header.id,offset});if(page.result_id!==header.id||JSON.stringify(page.key)!==JSON.stringify(header.key))throw new Error('Source evidence does not match the selected history.');return page;},onPeriod(value){period=value;void load(false);},onRefresh(){void load(true);},onCancel(){void cancelOwned().catch(e=>presentation.fail(errorText(e)));},async onDataPage(offset){if(!displayed)throw new Error('No saved history selected.');const header=displayed;const page=await api<PortfolioHistoryPage>({kind:'history_read',portfolio_id:header.key.portfolio_id,id:header.id,offset});validatePage(page,header);return page;}},initial);
 function validatePage(page:PortfolioHistoryPage,header:PortfolioHistoryHeader){if(page.result_id!==header.id||JSON.stringify(page.key)!==JSON.stringify(header.key))throw new Error('Saved history page does not match the selected result.');}
 async function cancelOwned(){
  const pending=starting;if(pending){try{const header=await pending;if(header.status==='running')owned={portfolioId:header.key.portfolio_id,id:header.id};}catch{}if(starting===pending)starting=null;}
  const previous=owned;owned=null;if(!previous)return;await api({kind:'history_cancel',portfolio_id:previous.portfolioId,id:previous.id});for(let i=0;i<50;i++){const status=await api<PortfolioHistoryHeader>({kind:'history_status',portfolio_id:previous.portfolioId,id:previous.id});if(status.status!=='running')return;await new Promise(resolve=>setTimeout(resolve,100));}}
 async function show(header:PortfolioHistoryHeader,origin:HistorySelection){
  const points:PerformancePoint[]=[];let offset=0;
  do{const page=await api<PortfolioHistoryPage>({kind:'history_read',portfolio_id:origin.portfolioId,id:header.id,offset});if(!active(origin))return;validatePage(page,header);for(const point of page.items){if(points.length&&point.date<=points.at(-1)!.date)throw new Error('Historical dates did not advance.');points.push(point);}if(points.length>100000)throw new Error('Historical data exceeds display limits.');if(page.next_offset===null)break;if(page.next_offset!==points.length||page.next_offset<=offset)throw new Error('History pagination did not advance.');offset=page.next_offset;}while(true);
  if(points.length!==header.row_count)throw new Error('Saved historical data is incomplete.');
  if(!active(origin))return;displayed=header;const stale=header.key.revision!==origin.revision;presentation.render(header,points,stale);presentation.setOnline(online);updateSummary(header.summary,period,stale);
 }
 async function load(force:boolean){
  if(!view||!current)return;current={...current,generation:current.generation+1,period,resultId:null};const origin={...current};const selected=view;
  try{
   await cancelOwned();if(!active(origin))return;presentation.clear();presentation.loading();presentation.setOnline(online);updateSummary(null,period,false);displayed=null;
   const nyToday=new Intl.DateTimeFormat('en-CA',{timeZone:'America/New_York',year:'numeric',month:'2-digit',day:'2-digit'}).format(new Date());const range=historyRange(period,selected.as_of<nyToday?selected.as_of:nyToday);const cached=await api<PortfolioHistoryHeader|null>({kind:'history_latest',portfolio_id:origin.portfolioId,account_id:origin.accountId,range});if(!active(origin))return;
   if(cached)await show(cached,origin);if(!active(origin))return;
   if(!online){if(!cached)presentation.fail('Offline · No saved history for this period.');presentation.setOnline(false);return;}
   presentation.loading();const pending=api<PortfolioHistoryHeader>({kind:'history_start',request:{request_id:crypto.randomUUID(),portfolio_id:origin.portfolioId,account_id:origin.accountId,expected_revision:origin.revision,range,refresh:force?'force':'missing'}});starting=pending;let header=await pending;if(starting===pending)starting=null;
   if(!active(origin)){if(header.status==='running')await api({kind:'history_cancel',portfolio_id:origin.portfolioId,id:header.id});return;}
   if(header.status==='running')owned={portfolioId:origin.portfolioId,id:header.id};
   while(header.status==='running'){await new Promise(resolve=>setTimeout(resolve,350));if(!active(origin))return;header=await api<PortfolioHistoryHeader>({kind:'history_status',portfolio_id:origin.portfolioId,id:header.id});}
   if(!active(origin))return;owned=null;
   if(header.status==='complete'||header.status==='partial')await show(header,origin);else presentation.fail(`${header.status.replaceAll('_',' ')}${header.error?`: ${header.error}`:''}${cached?' · Saved history remains available.':''}`);
  }catch(e){if(active(origin))presentation.fail(errorText(e));}
 }
 return{async select(next:View,accountId:string|null,isOnline:boolean){view=next;online=isOnline;current={portfolioId:next.id,accountId,revision:next.revision,generation:(current?.generation??0)+1,period,resultId:null};await load(false);},refresh:()=>load(true),period:()=>period,dispose(){alive=false;current=null;void cancelOwned().catch(()=>{});presentation.dispose();}};
}
