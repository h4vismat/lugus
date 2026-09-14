import type {PortfolioApi} from './api';
import type {Account,Instrument,Opening,OpeningLot,Receipt,View} from './types';
import {button,check,commandDialog,field,node,select,today} from './forms';
export function accountDialog(api:PortfolioApi,view:View,saved:(r:Receipt)=>Promise<void>,existing?:Account){
 commandDialog(existing?'Correct starting balances':'Add account',api,{portfolio_id:view.id,expected_revision:view.revision},form=>{
  const name=field(form,'Account name',existing?.name??'');if(existing)name.disabled=true;const start=field(form,'Start date',existing?.start??today(),'date');
  const mode=select(form,'Starting point',[{value:'full_history',label:'Enter full transaction history'},{value:'existing',label:'Start from existing holdings'}],existing?.opening?.kind??'full_history');
  const note=node('p');form.append(note);const opening=node('section');form.append(opening);
  const cash=field(opening,'Opening cash (USD)',existing?.opening?.kind==='existing'?existing.opening.cash:'0');
  const lots=node('div',undefined,'portfolio-lots');opening.append(lots);
  const readers:{read:()=>OpeningLot;row:HTMLElement}[]=[];
  const add=(original?:OpeningLot)=>{const row=node('fieldset');row.append(node('legend','Remaining purchase lot'));lots.append(row);
   const instrument=select(row,'Instrument',view.instruments.map(i=>({value:i.id,label:`${i.symbol} — ${i.name}`})),original?.instrument_id);
   const acquired=field(row,'Original purchase date',original?.acquired??start.value,'date');
   const qty=field(row,'Remaining shares',original?.quantity??'');const basis=field(row,'Remaining total cost basis, including fees (USD)',original?.basis??'');
   const simplified=check(row,'Aggregate basis only — simplified FIFO history',original?.simplified??false);const unknown=check(row,'Original purchase date unknown',original?.date_assumed??false);
   const lotId=original?.id??crypto.randomUUID();const entry={row,read:():OpeningLot=>({id:lotId,instrument_id:instrument.value,acquired:unknown.checked?start.value:acquired.value,tie_order:original?.tie_order??readers.indexOf(entry),quantity:qty.value,basis:basis.value,simplified:simplified.checked||unknown.checked,date_assumed:unknown.checked})};
   unknown.onchange=()=>{acquired.disabled=unknown.checked;if(unknown.checked)simplified.checked=true;};unknown.onchange(new Event('change'));
   row.append(button('Remove lot',()=>{readers.splice(readers.indexOf(entry),1);row.remove();row.dispatchEvent(new Event('input',{bubbles:true}));}));readers.push(entry);
  };
  opening.append(button('Add purchase lot',()=>{if(!view.instruments.length)throw new Error('Add an instrument before entering opening lots.');add();}));
  if(existing?.opening?.kind==='existing')existing.opening.lots.forEach(add);
  const update=()=>{opening.hidden=mode.value!=='existing';opening.querySelectorAll<HTMLInputElement>('input').forEach(i=>{i.required=!opening.hidden&&i.type!=='checkbox';});note.textContent=opening.hidden?'The account starts at zero. Enter deposits, purchases, sales and other transactions from this date in chronological order.':'Enter cash and the lots you still own at the start of this date. Opening balances do not count as new purchases or income.';};mode.onchange=update;update();
  return ()=>{const balance:Opening=mode.value==='existing'?{kind:'existing',cash:cash.value,lots:readers.map(r=>r.read())}:{kind:'full_history'};
   return existing?{kind:'change_setup',account_id:existing.id,start:start.value,opening:balance,edits:[]}:{kind:'create_account',name:name.value,start:start.value,opening:balance,events:[]};};
 },saved);
}
export function instrumentDialog(api:PortfolioApi,view:View,saved:(r:Receipt)=>Promise<void>){
 commandDialog('Add instrument',api,{portfolio_id:view.id,expected_revision:view.revision},form=>{
  const name=field(form,'Name');const symbol=field(form,'Ticker');const kind=select(form,'Asset type',[{value:'stock',label:'Stock'},{value:'etf',label:'ETF'}]);form.append(node('p','Currency: USD'));
  return()=>({kind:'create_instrument',name:name.value,symbol:symbol.value,asset_kind:kind.value});
 },saved);
}
export async function bindingDialog(api:PortfolioApi,view:View,instrument:Instrument,saved:(r:Receipt)=>Promise<void>){
 const providers=await api<{instance_id:string;name:string}[]>({kind:'providers'});if(!providers.length)throw new Error('No market-data provider is configured. Configure a provider in Lugus to refresh prices.');
 commandDialog(`Price source for ${instrument.symbol}`,api,{portfolio_id:view.id,expected_revision:view.revision},form=>{
  const provider=select(form,'Provider',providers.map(p=>({value:p.instance_id,label:`${p.name} (${p.instance_id})`})),instrument.binding?.instance_id);const namespace=field(form,'Provider symbol namespace',instrument.binding?.native_id.namespace??'yahoo:symbol');const symbol=field(form,'Provider symbol',instrument.binding?.native_id.value??instrument.symbol);
  return()=>({kind:'bind_instrument',instrument_id:instrument.id,instance_id:provider.value,native_id:{namespace:namespace.value,value:symbol.value}});
 },saved);
}
