import type {PortfolioApi} from './api';
import type {Account,Audit,Header,LotRow,Page,Receipt,Snapshot,TransactionRow,View} from './types';
import {acceptsPortfolioResult,type Selection} from './state';
import {button,commandDialog,errorText,field,node,select,today} from './forms';
import {accountDialog,bindingDialog,instrumentDialog} from './accounts';
import {transactionDialog,voidDialog} from './transactions';
import {renderOverview} from './overview';
import {renderHoldings} from './holdings';
import {createHistoryController} from './history-controller';
import {openHoldingDetail,type ResearchIntent} from './holding-detail';
import {historyRange,type Period} from './history-state';
import {formatUsd} from './format';
export function mountPortfolio(root:HTMLElement,api:PortfolioApi,onUse:(view:View,accountId:string|null,intent?:ResearchIntent)=>Promise<void>){
 let standalone:{id:string;portfolioId:string}|null=null;let pendingStandalone:Promise<{id:string;status:string;key:{portfolio_id:string}}>|null=null;
 async function stopStandalone(){const pending=pendingStandalone;if(pending){try{const r=await pending;if(r.status==='running')standalone={id:r.id,portfolioId:r.key.portfolio_id};}catch{}if(pendingStandalone===pending)pendingStandalone=null;}const job=standalone;standalone=null;if(job){await api({kind:'history_cancel',portfolio_id:job.portfolioId,id:job.id});for(let i=0;i<50;i++){const r=await api<{status:string}>({kind:'history_status',portfolio_id:job.portfolioId,id:job.id});if(r.status!=='running')break;await new Promise(resolve=>setTimeout(resolve,100));}}}
 let history:ReturnType<typeof createHistoryController>|null=null;let closeHolding:(()=>void)|null=null;let online=false;let period:Period='YTD';
 const dispose=()=>{if(history){period=history.period();history.dispose();history=null;}closeHolding?.();closeHolding=null;};
 let selection:Selection={portfolioId:null,accountId:null,generation:0};let section='overview';let view:View|null=null;let headers:Header[]=[];let allAccounts:Account[]=[];let visible=false;
 const hide=()=>{dispose();void stopStandalone().catch(()=>{});visible=false;selection={...selection,generation:selection.generation+1};document.body.classList.remove('portfolio-open');root.hidden=true;};
 const fail=(e:unknown)=>{const status=document.getElementById('portfolio-feedback');if(status)status.textContent=errorText(e);};
 const saved=async(r:Receipt)=>{selection={portfolioId:r.portfolio_id,accountId:null,generation:selection.generation+1};await load();};
 const create=()=>commandDialog('Create portfolio',api,{portfolio_id:null,expected_revision:'0'},form=>{const name=field(form,'Portfolio name','My portfolio');return()=>({kind:'create_portfolio',name:name.value});},saved);
 async function pages<T>(selected:Selection,kind:string):Promise<T[]>{const output:T[]=[];let offset=0;do{const page=await api<Page<T>>({kind:'rows',portfolio_id:selected.portfolioId,account_id:selected.accountId,section:kind,offset,revision:view?.revision??null});output.push(...page.items);if(page.next_offset===null)return output;if(page.next_offset<=offset)throw new Error('Saved data pagination did not advance.');offset=page.next_offset;if(output.length>10000)throw new Error('This view exceeds 10,000 rows. Choose an account to narrow the view.');}while(true);}
 async function load(){
  if(!visible)return;dispose();selection={...selection,generation:selection.generation+1};const origin={...selection};root.replaceChildren(node('p','Loading portfolio…'));
  try{
   await stopStandalone();if(!acceptsPortfolioResult(selection,origin))return;
   const first=await api<Page<Header>>({kind:'list',offset:0});headers=first.items;let next=first.next_offset;while(next!==null){const page=await api<Page<Header>>({kind:'list',offset:next});headers.push(...page.items);next=page.next_offset;}
   if(!acceptsPortfolioResult(selection,origin))return;
   if(!headers.length){const title=node('header');title.append(node('h1','Your portfolio'),button('Back to research',hide));root.replaceChildren(title,node('p','Track stocks, ETFs and cash in USD. Enter your transaction history or start with the holdings you already own.'),button('Create portfolio',create));return;}
   if(!selection.portfolioId||!headers.some(p=>p.id===selection.portfolioId)){selection.portfolioId=headers[0].id;origin.portfolioId=selection.portfolioId;}
   const whole=await api<View>({kind:'overview',portfolio_id:selection.portfolioId,account_id:null});allAccounts=whole.accounts;
   const loaded=selection.accountId?await api<View>({kind:'overview',portfolio_id:selection.portfolioId,account_id:selection.accountId}):whole;
   if(!acceptsPortfolioResult(selection,origin))return;view=loaded;
   const header=node('header',undefined,'portfolio-header');header.append(node('h1',loaded.name),button('Back to research',hide));
   const controls=node('div',undefined,'portfolio-actions');const picker=select(controls,'Portfolio',headers.map(p=>({value:p.id,label:p.name})),selection.portfolioId);picker.onchange=()=>{selection={portfolioId:picker.value,accountId:null,generation:selection.generation+1};void load();};
   const accounts=select(controls,'Account',[{value:'',label:'All accounts'},...allAccounts.map(a=>({value:a.id,label:a.name}))],selection.accountId??'');accounts.onchange=()=>{selection={...selection,accountId:accounts.value||null,generation:selection.generation+1};void load();};
   controls.append(button('New portfolio',create),button('Add transaction',async()=>{const events=await pages<TransactionRow>(origin,'transactions');if(!acceptsPortfolioResult(selection,origin))return;const next=events.filter(r=>r.event.date===today()).reduce((n,r)=>Math.max(n,r.event.order+1),0);transactionDialog(api,whole,selection.accountId,next,saved);}),button('Use in chat',()=>onUse(whole,selection.accountId)),button('Refresh prices',async()=>{
    const b=controls.querySelectorAll('button');b.forEach(x=>x.disabled=true);
    try{let result=await api<{id:string;status:string}>({kind:'refresh',request:{request_id:crypto.randomUUID(),portfolio_id:loaded.id,expected_revision:loaded.revision}});
     const cancel=button('Cancel refresh',async()=>{await api({kind:'refresh_cancel',id:result.id});});controls.append(cancel);
     try{while(result.status==='running'){await new Promise(resolve=>setTimeout(resolve,500));result=await api<{id:string;status:string}>({kind:'refresh_status',id:result.id});}}finally{cancel.remove();}
     if(acceptsPortfolioResult(selection,origin)){await load();if(history){await history.refresh();}else{
      const scope={...selection};const nyDay=new Intl.DateTimeFormat('en-CA',{timeZone:'America/New_York',year:'numeric',month:'2-digit',day:'2-digit'}).format(new Date());
      const pending=api<{id:string;status:string;key:{portfolio_id:string}}>({kind:'history_start',request:{request_id:crypto.randomUUID(),portfolio_id:loaded.id,account_id:scope.accountId,expected_revision:loaded.revision,range:historyRange(period,nyDay),refresh:'force'}});pendingStandalone=pending;
      let r=await pending;if(pendingStandalone===pending)pendingStandalone=null;
      if(r.status==='running')standalone={id:r.id,portfolioId:loaded.id};
      if(!acceptsPortfolioResult(selection,scope)){await stopStandalone();return;}
      const cancelHistory=button('Cancel history',stopStandalone);root.querySelector('.portfolio-actions')?.append(cancelHistory);
      try{while(r.status==='running'&&acceptsPortfolioResult(selection,scope)){await new Promise(resolve=>setTimeout(resolve,350));r=await api({kind:'history_status',portfolio_id:loaded.id,id:r.id});}}finally{cancelHistory.remove();if(standalone?.id===r.id)standalone=null;}
     }const status=document.getElementById('portfolio-feedback');if(status)status.textContent=result.status==='partial'?'Refresh finished with unpriced or unavailable observations. See price refresh details.':`Refresh: ${result.status}`;}}finally{b.forEach(x=>x.disabled=false);}
   }));
   const nav=node('nav',undefined,'portfolio-tabs');for(const name of ['overview','holdings','transactions','accounts','audit']){const tab=button(name[0].toUpperCase()+name.slice(1),()=>{section=name;return load();});tab.setAttribute('aria-current',String(section===name));nav.append(tab);}
   const feedback=node('p');feedback.id='portfolio-feedback';feedback.setAttribute('role','status');const content=node('div');root.replaceChildren(header,controls,nav,feedback,content);
   const openHolding=(id:string,trigger:HTMLElement)=>{closeHolding?.();closeHolding=openHoldingDetail(api,loaded,selection.accountId,id,trigger,intent=>onUse(whole,selection.accountId,intent));};
   if(section==='overview'){const overview=renderOverview(content,loaded,openHolding);history=createHistoryController(api,overview.performanceRoot,overview.updateSummary,period);void history.select(loaded,selection.accountId,online);}
   if(section==='holdings')renderHoldings(content,loaded,openHolding);
   if(section==='transactions'){
    const rows=await pages<TransactionRow>(origin,'transactions');if(!acceptsPortfolioResult(selection,origin))return;
    if(!rows.length)content.append(node('p','No transactions yet. Add a deposit before recording purchases in an account with full history.'));
    const table=node('table');const head=node('tr');for(const label of ['Date / order','Account','Activity','Details','Actions'])head.append(node('th',label));table.append(head);
    for(const row of rows){const e=row.event;const tr=node('tr');tr.append(node('td',`${e.date} / ${e.order}`),node('td',row.account_name),node('td',e.kind.kind));const instrument=loaded.instruments.find(i=>i.id===e.kind.instrument_id);tr.append(node('td',e.kind.quantity?`${e.kind.quantity} ${instrument?.symbol??''} · gross ${formatUsd(String(e.kind.gross))} · fees ${formatUsd(String(e.kind.fees))}`:e.kind.amount?formatUsd(String(e.kind.amount)):`${e.kind.numerator}:${e.kind.denominator} ${instrument?.symbol??''}`));const actions=node('td');actions.append(button('Correct',()=>transactionDialog(api,whole,row.account_id,e.order,saved,row)),button('Void',()=>voidDialog(api,whole,row,saved)));tr.append(actions);table.append(tr);}content.append(table);
   }
   if(section==='accounts'){
    content.append(button('Choose benchmark source',async()=>{const providers=await api<{instance_id:string;plugin_id:string;plugin_version:string}[]>({kind:'history_providers'});if(!acceptsPortfolioResult(selection,origin))return;commandDialog('S&P 500 total-return source',api,{portfolio_id:whole.id,expected_revision:whole.revision},form=>{const choice=select(form,'Benchmark provider',[{value:'',label:'Automatic (one compatible provider)'},...providers.map(p=>({value:p.instance_id,label:`${p.instance_id} · ${p.plugin_id} ${p.plugin_version}`}))],whole.benchmark_instance_id??'');return()=>({kind:'set_benchmark_provider',instance_id:choice.value||null});},saved);}));
    content.append(button('Add account',()=>accountDialog(api,whole,saved)),button('Add instrument',()=>instrumentDialog(api,whole,saved)));
    const accounts=await pages<Account>(origin,'accounts');if(!acceptsPortfolioResult(selection,origin))return;
    for(const a of accounts){const card=node('article',undefined,'portfolio-account');card.append(node('h3',a.name),node('p',`USD · starts ${a.start}`),button('Correct starting balances',()=>accountDialog(api,whole,saved,a)),button('Rename',()=>commandDialog('Rename account',api,{portfolio_id:whole.id,expected_revision:whole.revision},form=>{const name=field(form,'Name',a.name);return()=>({kind:'rename_account',account_id:a.id,name:name.value});},saved)));content.append(card);}
    content.append(node('h2','Instruments'));for(const i of whole.instruments){const line=node('article',undefined,'portfolio-account');line.append(node('h3',`${i.symbol} — ${i.name}`),node('p',`${i.asset_kind.toUpperCase()} · USD · ${i.binding?'Price source connected':'No price source'}`),button('Choose price source',()=>bindingDialog(api,whole,i,saved)));content.append(line);}
   }
   if(section==='audit'){
    const renderPage=async(offset:number)=>{const page=await api<Page<Audit>>({kind:'audit',portfolio_id:loaded.id,offset});if(!acceptsPortfolioResult(selection,origin))return;for(const row of page.items){const details=node('details');details.append(node('summary',`${row.recorded_at} · ${String(row.mutation.kind).replaceAll('_',' ')} · revision ${row.revision}`),node('pre',JSON.stringify(row.mutation,null,2)));content.append(details);}if(page.next_offset!==null){const more=button('Older activity',async()=>{more.remove();await renderPage(page.next_offset!);});content.append(more);}};await renderPage(0);
   }
  }catch(e){if(acceptsPortfolioResult(selection,origin)){if(!document.getElementById('portfolio-feedback')){const status=node('p');status.id='portfolio-feedback';status.setAttribute('role','status');root.replaceChildren(status,button('Retry',load),button('Back to research',hide));}fail(e);}}
 }
 return{setOnline(value:boolean){if(online!==value){online=value;if(visible)void load();}},show(){visible=true;document.body.classList.add('portfolio-open');root.hidden=false;void load();},hide};
}
