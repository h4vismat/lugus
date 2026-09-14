import type {PortfolioApi} from './api';
import type {Command,Preview,Receipt} from './types';
import {formatUsd} from './format';
export function node<K extends keyof HTMLElementTagNameMap>(tag:K,text?:string,className?:string):HTMLElementTagNameMap[K]{const n=document.createElement(tag);if(text!==undefined)n.textContent=text;if(className)n.className=className;return n;}
export function button(text:string,action:()=>void|Promise<void>):HTMLButtonElement{const b=node('button',text);b.type='button';b.onclick=()=>{Promise.resolve().then(action).catch(e=>{const status=b.closest('dialog')?.querySelector('[role="status"]')??document.getElementById('portfolio-feedback');if(status)status.textContent=errorText(e);});};return b;}
export function errorText(e:unknown){return typeof e==='object'&&e!==null&&'message'in e?String(e.message):String(e);}
export function field(parent:HTMLElement,label:string,value='',type='text'):HTMLInputElement{const wrap=node('label',label);const input=node('input');input.type=type;input.value=value;input.required=true;if(type==='text')input.maxLength=256;wrap.append(input);parent.append(wrap);return input;}
export function select(parent:HTMLElement,label:string,options:{value:string;label:string}[],selected?:string){const wrap=node('div');const caption=node('label',label);const input=node('select');input.id=`portfolio-select-${crypto.randomUUID()}`;caption.htmlFor=input.id;wrap.append(caption);for(const o of options){const opt=node('option',o.label);opt.value=o.value;input.append(opt);}if(selected!==undefined)input.value=selected;wrap.append(input);parent.append(wrap);return input;}
export function check(parent:HTMLElement,label:string,checked=false){const wrap=node('label',undefined,'portfolio-check');const input=node('input');input.type='checkbox';input.checked=checked;wrap.append(input,document.createTextNode(label));parent.append(wrap);return input;}
export function today(){const d=new Date();return `${d.getFullYear()}-${String(d.getMonth()+1).padStart(2,'0')}-${String(d.getDate()).padStart(2,'0')}`;}
export function orderValue(input:HTMLInputElement){const n=Number(input.value);if(!Number.isSafeInteger(n)||n<0)throw new Error('Same-day order must be a nonnegative whole number.');return n;}
export function commandDialog(title:string,api:PortfolioApi,envelope:{portfolio_id:string|null;expected_revision:string},build:(form:HTMLElement)=>()=>Record<string,unknown>,saved:(receipt:Receipt)=>Promise<void>){
 const dialog=node('dialog',undefined,'portfolio-dialog');const form=node('form');form.append(node('h2',title));const fields=node('div',undefined,'portfolio-fields');form.append(fields);const read=build(fields);
 const feedback=node('pre',undefined,'portfolio-preview');feedback.setAttribute('role','status');form.append(feedback);const controls=node('div',undefined,'portfolio-actions');const submit=node('button','Preview');submit.type='submit';controls.append(button('Cancel',()=>dialog.close()),submit);form.append(controls);dialog.append(form);document.body.append(dialog);
 let pending:Command|null=null;let fingerprint='';let busy=false;
 form.addEventListener('input',()=>{pending=null;submit.textContent='Preview';feedback.textContent='';});
 form.onsubmit=async event=>{event.preventDefault();if(busy)return;busy=true;submit.disabled=true;
  const wasReady=pending!==null;
  try{const mutation=read();const serialized=JSON.stringify(mutation);
   if(!pending||serialized!==fingerprint){pending={...envelope,request_id:crypto.randomUUID(),mutation};fingerprint=serialized;const preview=await api<Preview>({kind:'preview',request:pending});
    const matches=preview.states.flatMap(s=>s.matches);feedback.textContent=`After this change\nCash: ${formatUsd(preview.view.valuation.cash)}\nRealized P&L: ${formatUsd(preview.view.realized)}${matches.length?'\n\nFIFO sales:\n'+matches.map(m=>`${m.quantity} shares · basis ${formatUsd(m.basis)} · net proceeds ${formatUsd(m.net_proceeds)} · P&L ${formatUsd(m.realized)}${m.simplified?' (simplified opening history)':''}`).join('\n'):''}`;submit.textContent='Save';
   }else{const receipt=await api<Receipt>({kind:'execute',request:pending});dialog.close();await saved(receipt);}
  }catch(e){feedback.textContent=errorText(e);const conflict=typeof e==='object'&&e!==null&&'kind'in e&&String(e.kind).toLowerCase()==='conflict';if(!wasReady||conflict){pending=null;submit.textContent='Preview';}else{submit.textContent='Retry save';}}finally{busy=false;submit.disabled=false;}
 };
 dialog.addEventListener('close',()=>dialog.remove());dialog.showModal();fields.querySelector<HTMLInputElement>('input')?.focus();
}
