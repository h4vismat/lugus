export type AgentKind='codex'|'claude_code';
export interface AgentOption {id:AgentKind;label:string;available:boolean;detail:string}
export interface AgentSettings {selected:AgentKind;offline:boolean;runtime_available:boolean;agents:AgentOption[]}
export interface SettingsState {saved:AgentSettings|null;draft:AgentKind|null;status:'loading'|'ready'|'saving'|'error';notice:string}
export type SettingsAction={type:'loaded'|'saved';value:AgentSettings}|{type:'choose';agent:AgentKind}|{type:'saving'}|{type:'failed';message:string};
export const initialSettings=():SettingsState=>({saved:null,draft:null,status:'loading',notice:''});
export function settingsReducer(state:SettingsState,action:SettingsAction):SettingsState {
 switch(action.type){
  case 'loaded':return {saved:action.value,draft:action.value.selected,status:'ready',notice:''};
  case 'saved':return {saved:action.value,draft:action.value.selected,status:'ready',notice:'Saved. Your next message will use this agent. Active runs keep their current agent.'};
  case 'choose':return state.status==='saving'?state:{...state,draft:action.agent,notice:'',status:'ready'};
  case 'saving':return {...state,status:'saving',notice:'Saving…'};
  case 'failed':return {...state,status:'error',notice:action.message};
 }
}
export function canSaveSettings(state:SettingsState):boolean {
 return !!state.saved&&!state.saved.offline&&state.status!=='saving'&&state.status!=='loading'&&state.draft!==state.saved.selected&&state.saved.agents.some(a=>a.id===state.draft&&a.available);
}
