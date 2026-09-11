import {test} from 'node:test';
import assert from 'node:assert/strict';
import {initialSettings, settingsReducer, canSaveSettings, type AgentSettings} from './settings.ts';

const available: AgentSettings = {selected:'codex', offline:false, runtime_available:true, agents:[
 {id:'codex',label:'Codex',available:true,detail:'Installed'},
 {id:'claude_code',label:'Claude Code',available:true,detail:'Installed'},
]};
test('choosing an agent does not mark it saved until the native host confirms',()=>{
 let state=settingsReducer(initialSettings(),{type:'loaded',value:available});
 assert.equal(canSaveSettings(state),false);
 state=settingsReducer(state,{type:'choose',agent:'claude_code'});
 assert.equal(state.saved?.selected,'codex');
 assert.equal(canSaveSettings(state),true);
 state=settingsReducer(state,{type:'saving'});
 assert.equal(canSaveSettings(state),false);
 assert.equal(settingsReducer(state,{type:'choose',agent:'codex'}).draft,'claude_code');
 state=settingsReducer(state,{type:'saved',value:{...available,selected:'claude_code'}});
 assert.equal(state.saved?.selected,'claude_code');
 assert.match(state.notice,/next message/i);
 assert.equal(canSaveSettings(state),false);
});
test('failed saves retain the previous agent and allow a retry',()=>{
 let state=settingsReducer(initialSettings(),{type:'loaded',value:available});
 state=settingsReducer(state,{type:'choose',agent:'claude_code'});
 state=settingsReducer(state,{type:'saving'});
 state=settingsReducer(state,{type:'failed',message:'Could not save agent selection'});
 assert.equal(state.saved?.selected,'codex');
 assert.equal(state.draft,'claude_code');
 assert.equal(canSaveSettings(state),true);
 assert.equal(state.notice,'Could not save agent selection');
});
test('unavailable agents and offline mode cannot be saved',()=>{
 const missing={...available,agents:available.agents.map(a=>({...a,available:a.id==='codex'}))};
 let state=settingsReducer(initialSettings(),{type:'loaded',value:missing});
 state=settingsReducer(state,{type:'choose',agent:'claude_code'});
 assert.equal(canSaveSettings(state),false);
 state=settingsReducer(state,{type:'loaded',value:{...available,offline:true}});
 state=settingsReducer(state,{type:'choose',agent:'claude_code'});
 assert.equal(canSaveSettings(state),false);
});
