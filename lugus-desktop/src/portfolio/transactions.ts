import type {PortfolioApi} from './api';
import type {Event as LedgerEvent,Receipt,TransactionRow,View} from './types';
import {check,commandDialog,field,node,orderValue,select,today} from './forms';
import {decimalProduct} from './format';
export function transactionDialog(api:PortfolioApi,view:View,accountId:string|null,nextOrder:number,saved:(r:Receipt)=>Promise<void>,original?:TransactionRow){
 if(!view.accounts.length)throw new Error('Add an account first.');
 commandDialog(original?'Correct transaction':'Add transaction',api,{portfolio_id:view.id,expected_revision:view.revision},form=>{
  const account=select(form,'Account',view.accounts.map(a=>({value:a.id,label:a.name})),original?.account_id??accountId??undefined);if(original)account.disabled=true;
  const kind=select(form,'Activity',['deposit','buy','sell','withdrawal','dividend','fee','split'].map(k=>({value:k,label:k[0].toUpperCase()+k.slice(1)})),original?.event.kind.kind??'deposit');
  if(original&&original.event.kind.kind==='split')kind.disabled=true;
  const date=field(form,'Effective date',original?.event.date??today(),'date');const order=field(form,'Same-day order',String(original?.event.order??nextOrder),'number');order.min='0';order.step='1';
  const instrumentWrap=node('section');form.append(instrumentWrap);const instrument=select(instrumentWrap,'Instrument',view.instruments.map(i=>({value:i.id,label:`${i.symbol} — ${i.name}`})),original?.event.kind.instrument_id?String(original.event.kind.instrument_id):undefined);
  const money=node('section');form.append(money);const amount=field(money,'Amount (USD)',String(original?.event.kind.amount??''));
  const trade=node('section');form.append(trade);const quantity=field(trade,'Shares',String(original?.event.kind.quantity??''));const price=field(trade,'Execution price (USD)',String(original?.event.kind.price??''));const gross=field(trade,'Gross trade amount (USD)',String(original?.event.kind.gross??''));const fees=field(trade,'Fees (USD)',String(original?.event.kind.fees??'0'));const override=check(trade,'Use broker-reported gross amount',original?.event.kind.gross_overridden===true);
  const defaultGross=()=>{if(!override.checked&&quantity.value&&price.value){try{gross.value=decimalProduct(quantity.value,price.value);}catch{}}gross.readOnly=!override.checked;};quantity.oninput=defaultGross;price.oninput=defaultGross;override.onchange=defaultGross;defaultGross();
  const split=node('section');form.append(split);split.append(node('p','This action adjusts this instrument across all accounts in the portfolio.'));const numerator=field(split,'New shares',String(original?.event.kind.numerator??'2'),'number');const denominator=field(split,'Old shares',String(original?.event.kind.denominator??'1'),'number');
  const update=()=>{const k=kind.value;instrumentWrap.hidden=!['buy','sell','dividend','split'].includes(k);money.hidden=!['deposit','withdrawal','dividend','fee'].includes(k);trade.hidden=!['buy','sell'].includes(k);split.hidden=k!=='split';for(const section of [money,trade,split])section.querySelectorAll<HTMLInputElement>('input').forEach(i=>i.required=!section.hidden&&i.type!=='checkbox');};kind.onchange=update;update();
  const id=original?.event.id??crypto.randomUUID();
  return()=>{
   if(['buy','sell','dividend','split'].includes(kind.value)&&!instrument.value)throw new Error('Add and select an instrument first.');
   if(kind.value==='split'){
    const n=orderValue(numerator),d=orderValue(denominator);if(!n||!d)throw new Error('Split ratio must be positive.');return original?{kind:'replace_split',action_id:original.event.kind.action_id,date:date.value,order:orderValue(order),numerator:n,denominator:d}:{kind:'apply_split',instrument_id:instrument.value,date:date.value,order:orderValue(order),numerator:n,denominator:d};
   }
   const payload:LedgerEvent['kind']=['buy','sell'].includes(kind.value)?{kind:kind.value,instrument_id:instrument.value,quantity:quantity.value,price:price.value,gross:gross.value,fees:fees.value,gross_overridden:override.checked}:kind.value==='dividend'?{kind:'dividend',instrument_id:instrument.value,amount:amount.value}:{kind:kind.value,amount:amount.value};
   return{kind:'edit_events',account_id:account.value,edits:[{kind:original?'replace':'append',event:{id,date:date.value,order:orderValue(order),kind:payload}}]};
  };
 },saved);
}
export function voidDialog(api:PortfolioApi,view:View,row:TransactionRow,saved:(r:Receipt)=>Promise<void>){
 commandDialog('Void transaction',api,{portfolio_id:view.id,expected_revision:view.revision},form=>{form.append(node('p',`Remove this ${row.event.kind.kind} from accounting? Its original record remains in audit history. Linked stock splits are removed across all affected accounts. Later transactions will be revalidated.`));return()=>row.event.kind.kind==='split'?{kind:'void_split',action_id:row.event.kind.action_id}:{kind:'edit_events',account_id:row.account_id,edits:[{kind:'void',id:row.event.id}]};},saved);
}
