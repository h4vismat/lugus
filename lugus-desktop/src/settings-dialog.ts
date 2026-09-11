import {byId,element} from './dom';
import {initialSettings,settingsReducer,canSaveSettings,type AgentKind,type AgentSettings,type SettingsAction} from './settings';

export function wireAgentSettings(rpc:<T>(request:object)=>Promise<T>,onSaved:(value:AgentSettings)=>void){
 const dialog=byId<HTMLDialogElement>('settings');
 const select=byId<HTMLSelectElement>('settings-agent');
 const save=byId<HTMLButtonElement>('settings-save');
 const closeButtons=[byId<HTMLButtonElement>('settings-close'),byId<HTMLButtonElement>('settings-done')];
 let state=initialSettings();let generation=0;
 dialog.addEventListener('cancel',event=>{if(state.status==='saving')event.preventDefault();});
 dialog.querySelector('form')!.addEventListener('submit',event=>{if(state.status==='saving')event.preventDefault();});
 const message=(error:unknown)=>typeof error==='object'&&error!==null&&'message' in error?String(error.message):String(error);
 function render(){
  const options=state.saved?.agents??[];
  select.replaceChildren(...options.map(agent=>{const option=element('option',`${agent.label}${agent.available?'':' — unavailable'}`);option.value=agent.id;option.disabled=!agent.available;return option;}));
  if(state.draft)select.value=state.draft;
  for(const button of closeButtons)button.disabled=state.status==='saving';
  select.disabled=!state.saved||state.saved.offline||state.status==='saving';
  save.disabled=!canSaveSettings(state);save.textContent=state.status==='saving'?'Saving…':'Save agent';
  byId('settings-detail').textContent=options.find(a=>a.id===state.draft)?.detail??'Loading available agents…';
  byId('settings-feedback').textContent=state.notice;
  byId('settings-feedback').classList.toggle('settings-error',state.status==='error');
  byId('settings-runtime').textContent=state.saved?.offline?'Offline mode is enabled. Reopen Lugus without LUGUS_OFFLINE to use an agent.':'Choose the agent for your next message. Each agent uses its own existing sign-in.';
 }
 function update(action:SettingsAction){state=settingsReducer(state,action);render();}
 byId('settings-button').onclick=async()=>{
  const current=++generation;state=initialSettings();render();dialog.showModal();
  try{const value=await rpc<AgentSettings>({operation:'agent_settings'});if(current===generation)update({type:'loaded',value});}
  catch(e){if(current===generation)update({type:'failed',message:message(e)});}
 };
 select.onchange=()=>update({type:'choose',agent:select.value as AgentKind});
 save.onclick=async()=>{
  if(!canSaveSettings(state)||!state.draft)return;
  const agent=state.draft;const current=generation;update({type:'saving'});
  try{const value=await rpc<AgentSettings>({operation:'select_agent',agent});onSaved(value);if(current===generation)update({type:'saved',value});}
  catch(e){if(current===generation)update({type:'failed',message:message(e)});}
 };
}
