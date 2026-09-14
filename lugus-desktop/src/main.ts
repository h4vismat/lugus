import {mountPortfolio} from './portfolio/panel';
import {createPortfolioApi} from './portfolio/api';
import type {Snapshot as PortfolioSnapshot} from './portfolio/types';
import {readSavedWindow,readSavedFacts} from './data';
import {invoke} from '@tauri-apps/api/core';
import {byId,element,markdown} from './dom';
import {acceptsResult,type Selection} from './state';
import {renderResearch,viewLabel,type ResearchTab} from './research';
import type {Conversation,Message,Run,Page,Activity,Workspace,View,Dataset,Binding,ResearchView} from './types';
import './style.css';
import {wireAgentSettings} from './settings-dialog';

const rpc=<T>(request:object)=>{const {operation,...args}=request as {operation:string};return invoke<T>('research',{payload:JSON.stringify({op:operation,...args})});};
let selection:Selection={id:null,generation:0};
let chats:Conversation[]=[];let nextChats:number|null=0;
let receipts:View[]=[];let panelIssues:string[]=[];let researchLoad=0;
let messages:Message[]=[];let workspace:Workspace|null=null;let research:ResearchView[]=[];
let currentRun:Run|null=null;let runtimeAvailable=false;let infoReady=false;let submitting=false;
let portfolioContext:{id:string;conversation:string;date:string}|null=null;
let includeContext=true;let activeTab:ResearchTab='overview';let range='ALL';let layoutRevision='';
const monitors=new Map<string,{run:Run;text:string;activity:string;offset:number}>();
const drafts=new Map<string,string>();
const companyHints=new Map<string,string>();
const messageContainer=byId('messages');const emptyMessage=messageContainer.innerHTML;
const emptyResearch=byId('research-content').innerHTML;
let retrySubmission:{conversation:string;request:string;text:string;company_hint:string|null;selected:{kind:'view'|'portfolio';id:string}[]}|null=null;
const terminal=(run:Run)=>['completed','failed','interrupted'].includes(run.status);
const errorText=(error:unknown)=>typeof error==='object'&&error!==null&&'message' in error?String(error.message):String(error);
function error(message:string){const node=byId('error');node.textContent=message;node.hidden=!message;}
function title(){return chats.find(c=>c.id===selection.id)?.title??'New conversation';}
function updateControls(){
 const active=!!currentRun&&!terminal(currentRun);const input=byId<HTMLTextAreaElement>('message-input');
 byId<HTMLButtonElement>('send').disabled=!infoReady||!runtimeAvailable||submitting||active||!input.value.trim();
 byId('stop').hidden=!active;byId('run-status').textContent=submitting?'Sending…':active?'Researching…':currentRun?.status==='interrupted'?'Stopped':currentRun?.status==='failed'?'Needs attention':'';
 byId('chat-title').textContent=title();const context=byId('selected-context');context.hidden=!workspace?.selected_view_id||!includeContext;
 if(!context.hidden){context.replaceChildren(element('span','Including the selected research view'));const remove=element('button','×');remove.type='button';remove.setAttribute('aria-label','Remove selected view from message context');remove.onclick=()=>{includeContext=false;updateControls();};context.append(remove);}
}
function renderChats(){const nav=byId('chats');nav.replaceChildren(...chats.map(chat=>{const button=element('button');button.setAttribute('aria-current',String(chat.id===selection.id));button.append(element('span','◷','chat-icon'),element('span',chat.title));button.title=chat.title;button.onclick=()=>openChat(chat).catch(e=>error(errorText(e)));return button;}));byId('more-chats').hidden=nextChats===null;}
async function loadChats(append=false){const page=await rpc<Page<Conversation>>({operation:'list',offset:append?nextChats??0:0});nextChats=page.next_offset;chats=append?[...chats,...page.items.filter(c=>!chats.some(old=>old.id===c.id))]:[...page.items,...chats.filter(c=>!page.items.some(fresh=>fresh.id===c.id))];renderChats();}
async function pages<T>(operation:string,conversation:string):Promise<T[]>{let offset=0;const output:T[]=[];for(let n=0;n<100;n++){const page=await rpc<Page<T>>({operation,conversation,offset});output.push(...page.items);if(page.next_offset===null)return output;if(page.next_offset<=offset)throw new Error('Saved history pagination did not advance.');offset=page.next_offset;}throw new Error('This conversation exceeds the desktop history limit.');}
function renderMessages(){
 const wasNearBottom=messageContainer.scrollHeight-messageContainer.scrollTop-messageContainer.clientHeight<100;const oldTop=messageContainer.scrollTop;
 if(!messages.length&&!currentRun){messageContainer.innerHTML=emptyMessage;wireSuggestions();return;}
 const nodes:HTMLElement[]=[];
 for(const message of messages){const row=element('article',undefined,`message ${message.role}`);if(message.role==='assistant')row.append(element('div','L','assistant-mark'));const body=element('div',undefined,'message-body');if(message.role==='assistant')body.append(markdown(message.text));else body.textContent=message.text;row.append(body);nodes.push(row);}
 const monitor=currentRun?monitors.get(currentRun.id):undefined;
 if(currentRun&&!terminal(currentRun)){if(monitor?.text){const row=element('article',undefined,'message assistant streaming');row.append(element('div','L','assistant-mark'));const body=element('div',undefined,'message-body');body.append(markdown(monitor.text));row.append(body);nodes.push(row);}nodes.push(element('div',monitor?.activity||'Researching your question…','activity'));}
 messageContainer.replaceChildren(...nodes);messageContainer.scrollTop=wasNearBottom?messageContainer.scrollHeight:oldTop;
}
async function loadMessages(origin:Selection){if(!origin.id)return;const result=await pages<Message>('messages',origin.id);if(!acceptsResult(selection,origin))return;messages=result;renderMessages();}
function drawResearch(){
 renderResearch(research,workspace?.selected_view_id??null,activeTab,range,value=>{range=value;saveViewPreferences();drawResearch();});
 const selected=receipts.find(v=>v.id===workspace?.selected_view_id);
 if(selected?.kind==='document')byId('research-content').replaceChildren(element('p','A saved filing document is selected as message context. Document reading is available through the agent.','evidence-note'));
 for(const issue of panelIssues)byId('research-content').append(element('p',issue,'evidence-note'));
}
function viewPreferenceKey(){return workspace?.selected_view_id?`lugus:view:${workspace.selected_view_id}`:null;}
function saveViewPreferences(){const key=viewPreferenceKey();if(key)try{localStorage.setItem(key,JSON.stringify({range,tab:activeTab}));}catch{ /* Data and chat persistence do not depend on localStorage. */ }}
function restoreViewPreferences(){const key=viewPreferenceKey();try{const saved=JSON.parse(key?localStorage.getItem(key)??'null':'null');range=['1M','6M','1Y','ALL'].includes(saved?.range)?saved.range:'ALL';activeTab=['overview','financials','filings'].includes(saved?.tab)?saved.tab:'overview';}catch{range='ALL';activeTab='overview';}for(const tab of ['overview','financials','filings'])byId(`tab-${tab}`).setAttribute('aria-selected',String(tab===activeTab));}
async function loadResearch(origin:Selection,force=false){
 if(!origin.id)return;const ticket=++researchLoad;
 const state=await rpc<Workspace>({operation:'workspace',conversation:origin.id});
 const current=()=>acceptsResult(selection,origin)&&ticket===researchLoad;
 if(!current())return;const key=`${origin.id}:${state.revision}`;if(key===layoutRevision&&!force)return;
 const cache=new Map(research.map(item=>[item.view.id,item]));const results:ResearchView[]=[];const views:View[]=[];const issues:string[]=[];
 for(let start=0;start<state.view_ids.length;start+=4){
  const batch=await Promise.all(state.view_ids.slice(start,start+4).map(async id=>{
   try{const view=await rpc<View>({operation:'view',conversation:origin.id,view:id});views.push(view);if(view.kind==='document')return null;
    if(cache.has(id)&&!force)return cache.get(id)!;
    const first=await rpc<Dataset>({operation:'read',conversation:origin.id,view:id,offset:0});
    const read=async(offset:number)=>{if(!current())throw new Error('Research selection changed.');return rpc<Dataset>({operation:'read',conversation:origin.id,view:id,offset});};
    const data=await (first.header.kind==='facts'?readSavedFacts:readSavedWindow)(first,read);
    let binding:Binding|undefined;if(data.header.binding_id)try{binding=await rpc<Binding>({operation:'binding',conversation:origin.id,binding:data.header.binding_id});}catch(e){issues.push(`Company identity unavailable: ${errorText(e)}`);}
    return {view,data,binding};
   }catch(e){issues.push(`Saved research unavailable: ${errorText(e)}`);return null;}
  }));results.push(...batch.filter((item):item is ResearchView=>item!==null));if(!current())return;
 }
 const oldSelected=workspace?.selected_view_id;workspace=state;research=results;receipts=views.sort((a,b)=>state.view_ids.indexOf(a.id)-state.view_ids.indexOf(b.id));panelIssues=issues;layoutRevision=key;
 if(oldSelected!==state.selected_view_id)restoreViewPreferences();
 const picker=byId<HTMLSelectElement>('research-view');picker.replaceChildren(...receipts.map(view=>{const item=research.find(r=>r.view.id===view.id);const option=element('option',item?viewLabel(item,research):view.kind==='document'?'Saved filing document':'Unavailable research');option.value=view.id;option.selected=view.id===state.selected_view_id;return option;}));byId('view-picker').hidden=receipts.length<2;
 if(!state.view_ids.length){byId('research-content').innerHTML=emptyResearch;byId('company-name').textContent='Company research';byId('company-listing').textContent='Charts & financial information';}else drawResearch();updateControls();
 const shown=research.find(v=>v.view.id===state.selected_view_id);if(shown)void rpc({operation:'presented',conversation:origin.id,view:shown.view.id,revision:shown.view.descriptor_revision,status:'presented'}).catch(()=>{});
}
async function openChat(chat:Conversation){
 drafts.set(selection.id??'new',byId<HTMLTextAreaElement>('message-input').value);companyHints.set(selection.id??'new',byId<HTMLInputElement>('company-hint').value);
 selection={id:chat.id,generation:selection.generation+1};const origin={...selection};messages=[];workspace=null;research=[];receipts=[];panelIssues=[];layoutRevision='';currentRun=null;includeContext=true;
 byId<HTMLTextAreaElement>('message-input').value=drafts.get(chat.id)??'';byId<HTMLInputElement>('company-hint').value=companyHints.get(chat.id)??'';error('');renderChats();updateControls();messageContainer.replaceChildren(element('p','Opening conversation…','evidence-note'));byId('research-content').innerHTML=emptyResearch;
 const [history,runs]=await Promise.all([pages<Message>('messages',chat.id),pages<Run>('runs',chat.id)]);if(!acceptsResult(selection,origin))return;
 messages=history;currentRun=runs.find(r=>!terminal(r))??runs.at(-1)??null;renderMessages();messageContainer.scrollTop=messageContainer.scrollHeight;
 if(currentRun&&!terminal(currentRun))startMonitor(currentRun,chat.id);else if(currentRun?.status==='failed')error(currentRun.error?.message??'The last research turn failed. You can send a new message to continue.');
 await loadResearch(origin,true);if(!acceptsResult(selection,origin))return;updateControls();try{localStorage.setItem('lugus:last-chat',chat.id);}catch{}
}
function newChat(){drafts.set(selection.id??'new',byId<HTMLTextAreaElement>('message-input').value);companyHints.set(selection.id??'new',byId<HTMLInputElement>('company-hint').value);selection={id:null,generation:selection.generation+1};messages=[];research=[];receipts=[];panelIssues=[];workspace=null;layoutRevision='';currentRun=null;includeContext=true;retrySubmission=null;byId<HTMLTextAreaElement>('message-input').value=drafts.get('new')??'';byId<HTMLInputElement>('company-hint').value=companyHints.get('new')??'';byId('research-content').innerHTML=emptyResearch;byId('company-name').textContent='Company research';byId('company-listing').textContent='Charts & financial information';byId('view-picker').hidden=true;error('');renderMessages();renderChats();updateControls();byId('message-input').focus();}
const toolLabel=(name:string)=>name.includes('resolve')||name.includes('lookup')?'Finding company information…':name.includes('bind')?'Confirming the market instrument…':name.includes('fetch')||name==='get_price_chart'?'Retrieving source data…':name.includes('open_view')?'Opening research alongside the conversation…':name.includes('dataset')?'Inspecting financial evidence…':name.includes('passage')||name.includes('text')?'Reading filing evidence…':'Working with research evidence…';
function startMonitor(run:Run,conversation:string){
 if(monitors.has(run.id))return;const monitor={run,text:'',activity:'Researching your question…',offset:0};monitors.set(run.id,monitor);
 const poll=async()=>{try{
  const [status,events]=await Promise.all([rpc<Run>({operation:'status',conversation,run:run.id}),rpc<Page<Activity>>({operation:'activity',conversation,run:run.id,offset:monitor.offset})]);monitor.run=status;
  for(const item of events.items){monitor.offset++;if(item.kind==='preparation'){try{const phase=JSON.parse(item.data).phase;monitor.activity=phase==='interpreting'?'Understanding your question…':phase==='retrieving'?'Preparing source evidence…':'Analyzing prepared evidence…';}catch{}continue;}if(item.kind!=='runtime')continue;try{const event=JSON.parse(item.data);if(event.type==='text_delta'&&typeof event.text==='string')monitor.text=(monitor.text+event.text).slice(0,65536);if(event.type==='tool_started')monitor.activity=toolLabel(String(event.name));}catch{}}
  if(selection.id===conversation){currentRun=status;renderMessages();updateControls();await loadResearch({...selection}).catch(e=>{if(selection.id===conversation)error(`Research panel unavailable: ${errorText(e)}`);});}
  if(terminal(status)){monitors.delete(run.id);if(selection.id===conversation){await loadMessages({...selection});if(status.status==='failed')error(status.error?.message??'The agent could not complete this request.');if(status.status==='interrupted')error('Research stopped. Saved evidence is still available.');updateControls();}await loadChats();return;}
  setTimeout(poll,events.next_offset===null?700:0);
 }catch(e){monitors.delete(run.id);if(selection.id===conversation)error(`Could not read agent progress: ${errorText(e)}. Reopen this chat to reconnect; the turn is not replayed.`);}};
 void poll();
}
async function sendMessage(event:Event){event.preventDefault();const input=byId<HTMLTextAreaElement>('message-input');const submittedDraft=input.value;const company_hint=byId<HTMLInputElement>('company-hint').value.trim()||null;const text=submittedDraft.trim();if(!text||submitting||!runtimeAvailable||(currentRun&&!terminal(currentRun)))return;submitting=true;error('');updateControls();const origin={...selection};
 try{let conversation=origin.id;
  if(!conversation){const created=await rpc<Conversation>({operation:'create',request:crypto.randomUUID(),title:Array.from(text.replace(/\s+/g,' ')).slice(0,72).join('')});conversation=created.id;chats.unshift(created);if(!acceptsResult(selection,origin))return;selection={id:conversation,generation:selection.generation+1};companyHints.set(conversation,byId<HTMLInputElement>('company-hint').value);companyHints.delete('new');renderChats();}
  const submissionOrigin={...selection};
  const chosen:{kind:'view'|'portfolio';id:string}[]=includeContext&&workspace?.selected_view_id?[{kind:'view',id:workspace.selected_view_id}]:[];
  if(portfolioContext?.conversation===conversation)chosen.push({kind:'portfolio',id:portfolioContext.id});
  const submission=retrySubmission?.conversation===conversation&&retrySubmission.text===text&&retrySubmission.company_hint===company_hint&&JSON.stringify(retrySubmission.selected)===JSON.stringify(chosen)?retrySubmission:{conversation,request:crypto.randomUUID(),text,company_hint,selected:chosen};retrySubmission=submission;
  const run=await rpc<Run>({operation:'send',...submission});retrySubmission=null;
  if(acceptsResult(selection,submissionOrigin)){if(input.value===submittedDraft){input.value='';drafts.delete(conversation);if(origin.id===null)drafts.delete('new');}currentRun=run;await loadMessages({...selection});messageContainer.scrollTop=messageContainer.scrollHeight;updateControls();}
  if(!terminal(run))startMonitor(run,conversation);await loadChats();
 }catch(e){if(selection.id===origin.id||origin.id===null)error(errorText(e));}finally{submitting=false;updateControls();}}
function wireSuggestions(){for(const button of document.querySelectorAll<HTMLButtonElement>('.suggestions button'))button.onclick=()=>{byId<HTMLTextAreaElement>('message-input').value=button.textContent??'';byId('message-input').focus();updateControls();};}
byId('new-chat').onclick=newChat;byId('more-chats').onclick=()=>loadChats(true).catch(e=>error(errorText(e)));byId('composer').onsubmit=sendMessage;
byId<HTMLTextAreaElement>('message-input').oninput=updateControls;
byId<HTMLInputElement>('company-hint').oninput=()=>{companyHints.set(selection.id??'new',byId<HTMLInputElement>('company-hint').value);retrySubmission=null;};
byId('message-input').onkeydown=event=>{if(event.key==='Enter'&&!event.shiftKey&&!event.isComposing){event.preventDefault();byId<HTMLFormElement>('composer').requestSubmit();}};
byId('stop').onclick=async()=>{if(!selection.id||!currentRun)return;try{const run=await rpc<Run>({operation:'cancel',conversation:selection.id,run:currentRun.id});if(run.conversation_id===selection.id){currentRun=run;updateControls();}}catch(e){error(errorText(e));}};
byId<HTMLSelectElement>('research-view').onchange=async event=>{if(!selection.id||!workspace)return;selection={...selection,generation:selection.generation+1};const origin={...selection};const view=(event.target as HTMLSelectElement).value;try{await rpc({operation:'select',conversation:origin.id,view,revision:workspace.revision});if(acceptsResult(selection,origin)){includeContext=true;await loadResearch(origin,true);}}catch(e){error(errorText(e));}};
for(const [index,tab] of (['overview','financials','filings'] as ResearchTab[]).entries()){const button=byId(`tab-${tab}`);button.onclick=()=>{activeTab=tab;for(const t of ['overview','financials','filings'])byId(`tab-${t}`).setAttribute('aria-selected',String(t===tab));saveViewPreferences();drawResearch();};button.onkeydown=e=>{if(['ArrowLeft','ArrowRight','Home','End'].includes(e.key)){e.preventDefault();const keys=['overview','financials','filings'];const next=e.key==='Home'?0:e.key==='End'?2:(index+(e.key==='ArrowRight'?1:2))%3;byId(`tab-${keys[next]}`).focus();byId(`tab-${keys[next]}`).click();}};}
const resize=byId('resize');function panelWidth(width:number){const bounded=Math.max(320,Math.min(640,width,window.innerWidth-580));document.documentElement.style.setProperty('--research-width',`${bounded}px`);resize.setAttribute('aria-valuenow',String(bounded));try{localStorage.setItem('lugus:research-width',String(bounded));}catch{}}
resize.onpointerdown=e=>{resize.setPointerCapture(e.pointerId);};resize.onpointermove=e=>{if(resize.hasPointerCapture(e.pointerId))panelWidth(window.innerWidth-e.clientX);};resize.onkeydown=e=>{if(e.key==='ArrowLeft'||e.key==='ArrowRight'){e.preventDefault();panelWidth(Number(resize.getAttribute('aria-valuenow'))+(e.key==='ArrowLeft'?20:-20));}};
wireAgentSettings(rpc,value=>{
 runtimeAvailable=value.runtime_available;infoReady=true;
 byId('connection').textContent=runtimeAvailable?`${value.agents.find(a=>a.id===value.selected)?.label??'Agent'} ready`:'Agent unavailable';
 if(runtimeAvailable)error('');updateControls();
});
async function initialize(){
 try{panelWidth(Number(localStorage.getItem('lugus:research-width'))||410);}catch{}
 const info=await rpc<{offline:boolean;runtime_available:boolean;agent?:string;startup_error?:string}>({operation:'info'});runtimeAvailable=info.runtime_available;infoReady=true;
 byId('connection').textContent=info.offline?'Offline':runtimeAvailable?(info.agent==='claude_code'?'Claude Code ready':info.agent==='codex'?'Codex ready':'Agent ready'):'Agent not configured';
 byId('settings-runtime').textContent=runtimeAvailable?'Agent runtime configured. A fresh session starts when you send a message.':'Agent runtime is not configured. Saved conversations and research remain available.';
 byId('settings-detail').textContent='Select Codex or Claude Code in Settings. Each uses its own existing sign-in.';
 if(info.startup_error)error(info.startup_error);else if(!runtimeAvailable)error('Select an installed agent in Settings to start chatting.');
 await loadChats();let last:string|null=null;try{last=localStorage.getItem('lugus:last-chat');}catch{}let chat=chats.find(c=>c.id===last);if(!chat&&last){try{chat=await rpc<Conversation>({operation:'conversation',conversation:last});chats.push(chat);renderChats();}catch{}}chat??=chats[0];if(chat)await openChat(chat);else wireSuggestions();updateControls();
}
initialize().catch(e=>{error(errorText(e));byId('connection').textContent='Connection unavailable';byId('settings-detail').textContent=errorText(e);});

const portfolioApi=createPortfolioApi(rpc);
const portfolioPanel=mountPortfolio(byId('portfolio'),portfolioApi,async(view,accountId)=>{
 let conversation=selection.id;
 if(!conversation){const created=await rpc<Conversation>({operation:'create',request:crypto.randomUUID(),title:'Portfolio review'});chats.unshift(created);renderChats();await openChat(created);conversation=created.id;}
 const snapshot=await portfolioApi<PortfolioSnapshot>({kind:'snapshot',request:{request_id:crypto.randomUUID(),portfolio_id:view.id,account_id:accountId,expected_revision:view.revision,conversation_id:conversation}});
 portfolioContext={id:snapshot.id,conversation,date:snapshot.summary.as_of};portfolioPanel.hide();
 byId<HTMLInputElement>('company-hint').value='';renderPortfolioContext();byId<HTMLTextAreaElement>('message-input').focus();
});
function renderPortfolioContext(){
 let context=document.getElementById('portfolio-context');if(!context){context=document.createElement('div');context.id='portfolio-context';byId('composer').prepend(context);}
 context.replaceChildren();context.hidden=!portfolioContext||portfolioContext.conversation!==selection.id;
 if(!context.hidden&&portfolioContext){context.append(element('span',`Portfolio snapshot as of ${portfolioContext.date} will be shared with the selected agent. Add a company hint for company research.`));const remove=element('button','Remove portfolio');remove.type='button';remove.onclick=()=>{portfolioContext=null;retrySubmission=null;renderPortfolioContext();};context.append(remove);}
}
byId('open-portfolio').onclick=()=>portfolioPanel.show();
byId('chats').addEventListener('click',()=>{portfolioPanel.hide();portfolioContext=null;renderPortfolioContext();});
byId('new-chat').addEventListener('click',()=>{portfolioPanel.hide();portfolioContext=null;renderPortfolioContext();});
