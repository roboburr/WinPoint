// The shell picker and desktop portal share the real Explorer and Rust file APIs.
import { h, loadCss } from '../../core/dom.js';
import { bridge } from '../../core/bridge.js';
import { makeT, onLangChange } from '../../core/i18n.js';
import { Button, Dropdown, Dialog, Checkbox } from '../../core/components.js';
import * as U from './util.js';
/** Portal glob rules are case-sensitive, including explicit bracket classes. */
export function matchPickerGlob(pattern,value,caseSensitive=false){
 let source='';
 const escape=char=>/[\\^$.*+?()[\]{}|]/.test(char)?'\\'+char:char;
 for(let i=0;i<pattern.length;i++){
  const char=pattern[i];
  if(char==='*'){source+='.*';continue;}if(char==='?'){source+='.';continue;}
  if(char==='['){
   let end=i+1;if(pattern[end]==='!'||pattern[end]==='^')end++;if(pattern[end]===']')end++;
   end=pattern.indexOf(']',end);
   if(end>i+1){let cls=pattern.slice(i+1,end),negate='';if(cls[0]==='!'||cls[0]==='^'){negate='^';cls=cls.slice(1);}cls=cls.replace(/\\/g,'\\\\').replace(/\[/g,'\\[').replace(/\]/g,'\\]');const token='['+negate+cls+']';try{new RegExp(token,'u');source+=token;i=end;continue;}catch{}}
  }
  source+=escape(char);
 }
 try{return new RegExp('^'+source+'$',caseSensitive?'u':'iu').test(value);}catch{return false;}
}
const t=makeT({nl:{open:'Openen',save:'Opslaan',saveAs:'Opslaan als',folder:'Map kiezen',cancel:'Annuleren',name:'Bestandsnaam',type:'Bestandstype',all:'Alle bestanden',required:'Geef een bestandsnaam op.',missing:'Dit bestand bestaat niet.',overwrite:'Bestand vervangen?',replace:'Vervangen',exists:'“{name}” bestaat al. Wil je het vervangen?',choose:'Kies een bestaande map.',file:'Kies een bestand.',readOnly:'Alleen-lezen openen'},en:{open:'Open',save:'Save',saveAs:'Save as',folder:'Choose a folder',cancel:'Cancel',name:'File name',type:'File type',all:'All files',required:'Enter a file name.',missing:'This file doesn’t exist.',overwrite:'Replace file?',replace:'Replace',exists:'“{name}” already exists. Do you want to replace it?',choose:'Choose an existing folder.',file:'Choose a file.',readOnly:'Open read-only'}});
export async function attachPicker(app,options){
 await loadCss('picker');
 const folder=options.folder===true,save=!folder&&(options.save===true||options.action==='save'),multiple=options.multiple===true&&!save;
 const filters=(Array.isArray(options.filters)?options.filters:[]).filter(f=>f&&typeof f==='object').slice(0,32);
 if(!filters.length)filters.push({name:t('all'),patterns:['*']});
 let filter=Math.max(0,Math.min(Number(options.currentFilter)||0,filters.length-1)),busy=false,typed=false;
 const name=h('input',{class:'fl-textbox xp-picker__name',type:'text',value:options.name||options.fileName||'',autocomplete:'off',spellcheck:false});
 const nameLabel=h('label',{class:'xp-picker__label'},h('span',{},t('name')),name);
 const types=Dropdown({options:filters.map((f,i)=>({value:String(i),label:f.name||t('all')})),value:String(filter),width:300,onChange:id=>{filter=Number(id)||0;app.refresh();}});
 const typeLabel=h('div',{class:'xp-picker__label'},h('span',{},t('type')),types);
 const buttonLabel=()=>options.acceptLabel||(folder?t('folder'):save?t('save'):t('open'));
 const choose=Button({label:buttonLabel(),variant:'accent',onClick:()=>submit()});
 const cancel=Button({label:t('cancel'),onClick:()=>post('close')});
 const error=h('div',{class:'xp-picker__error',role:'alert'});
 const choices=new Map(),controls=[];
 for(const choice of (Array.isArray(options.choices)?options.choices:[]).slice(0,32)){
  if(!choice||typeof choice.id!=='string')continue;
  const items=Array.isArray(choice.options)?choice.options:[];
  if(!items.length){choices.set(choice.id,choice.selected==='true'?'true':'false');controls.push(Checkbox({label:choice.label||choice.id,checked:choices.get(choice.id)==='true',onChange:value=>choices.set(choice.id,String(value))}));}
  else{const selected=String(choice.selected??items[0].id);choices.set(choice.id,selected);controls.push(h('div',{class:'xp-picker__label'},h('span',{},choice.label||choice.id),Dropdown({options:items.map(i=>({value:i.id,label:i.label})),value:selected,width:300,onChange:value=>choices.set(choice.id,value)})));}
 }
 let writable=options.writable!==false;
 const readOnly=options.allowReadOnly?Checkbox({label:t('readOnly'),checked:!writable,onChange:value=>{writable=!value;}}):null;
 const footer=h('footer',{class:'xp-picker__footer'},h('div',{class:'xp-picker__fields'},nameLabel,typeLabel,...controls,readOnly,error),h('div',{class:'xp-picker__buttons'},choose,cancel));
 app.frame.el.classList.add('xp-picker');app.frame.el.querySelector('.xp').append(footer);
 const title=()=>options.title||(folder?t('folder'):save?t('saveAs'):t('open'));
 app.pickerTitle=title;app.frame.setTitle(title());nameLabel.hidden=folder;typeLabel.hidden=folder;
 function post(cmd,data={}){window.ipc?.postMessage(JSON.stringify({cmd,...data}));}
 function matches(item){
  if(item.isDir||item.volume)return true;if(folder)return false;
  const f=filters[filter];const conditions=Array.isArray(f.conditions)?f.conditions:(f.patterns||['*']).map(value=>({type:String(value).includes('/')?1:0,value}));
  return conditions.some(({type,value})=>{
   const pattern=String(value);
   if(type===1)return pattern.endsWith('/*')?String(item.mime||'').startsWith(pattern.slice(0,-1)):item.mime===pattern;
   if(pattern==='*'||(!f.caseSensitive&&pattern==='*.*'))return true;
   return matchPickerGlob(pattern,item.name||'',f.caseSensitive===true);
  });
 }
 app.pickerFilter=matches;
 app.pickerSelection=items=>{typed=false;if(items.length===1&&!items[0].isDir&&!items[0].volume)name.value=items[0].name||U.basename(items[0].path);else if(multiple&&items.length>1)name.value=items.map(i=>`“${i.name||U.basename(i.path)}”`).join(' ');};
 app.pickerSelect=item=>{name.value=item.name||U.basename(item.path);submit([item.path]);};
 async function submit(explicit){
  if(busy)return;error.textContent='';busy=true;choose.disabled=true;
  try{
   const loc=app.tab?.loc;let paths=explicit;
   if(!paths&&multiple&&!typed){const selected=app.selectedItems().filter(i=>folder?(i.isDir||i.volume):!i.isDir&&!i.volume);if(selected.length)paths=selected.map(i=>i.volume?.mount||i.path);}
   if(!paths&&folder){const selected=app.selectedItems().find(i=>i.isDir||i.volume);const path=selected?.volume?.mount||selected?.path||(loc?.kind==='folder'?loc.path:null);if(path)paths=[path];}
   if(!paths&&!folder){const value=name.value;if(!value||value.includes('\0'))throw new Error(t('required'));const path=value.startsWith('/')?U.normPath(value):loc?.kind==='folder'?U.joinPath(loc.path,value):null;if(path)paths=[path];}
   if(!paths?.length)throw new Error(t('choose'));
   if(!multiple&&paths.length!==1)paths=paths.slice(0,1);
   for(const path of paths){
    let stat=null;try{stat=await bridge.call('files.stat',{path});}catch(e){if(e.code!=='not_found')throw e;}
    if(folder){if(!stat?.isDir)throw new Error(t('choose'));}
    else if(stat?.isDir){app.navigate({kind:'folder',path},{focus:true});name.value='';typed=false;return;}
    else if(!save&&!stat)throw new Error(t('missing'));
    if(save){const parent=await bridge.call('files.stat',{path:U.dirname(path)});if(!parent.isDir)throw new Error(t('choose'));if(stat&&!await Dialog.confirm({title:t('overwrite'),message:t('exists',{name:U.basename(path)}),confirmLabel:t('replace'),cancelLabel:t('cancel')}))return;}
   }
   post('picker-submit',{path:paths[0],paths,filterIndex:filter,choices:[...choices],writable});
  }catch(e){error.textContent=e?.message||String(e);}finally{busy=false;choose.disabled=false;}
 }
 name.addEventListener('input',()=>{typed=true;});
 name.addEventListener('keydown',event=>{if(event.key==='Enter'&&!event.isComposing){event.preventDefault();event.stopPropagation();submit();}});
 document.addEventListener('keydown',event=>{if(event.key==='Escape'&&!document.querySelector('.fl-dialog-layer')&&!app.view?.renaming){event.preventDefault();event.stopImmediatePropagation();post('close');}},true);
 addEventListener('win11:picker-error',event=>{error.textContent=event.detail?.message||'';});
 onLangChange(()=>{nameLabel.firstElementChild.textContent=t('name');typeLabel.firstElementChild.textContent=t('type');choose.setLabel(buttonLabel());cancel.setLabel(t('cancel'));app.frame.setTitle(title());});
 return footer;
}
