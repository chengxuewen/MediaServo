// P7.4 UI assertion harness (C61): Marionette-driven Firefox headless.
// Read-back channel = WebDriver:ExecuteScript returning JSON (no OCR / no exit-code trust).
// Products: docs/ui/prototypes/.harness/<case>.png (stat>0 enforced) + results.json.
// Run: node scripts/ui-harness.mjs   (requires firefox on PATH; serves prototype over local HTTP)
import net from 'node:net';
import http from 'node:http';
import fs from 'node:fs';
import path from 'node:path';
import { spawn } from 'node:child_process';
import os from 'node:os';
import { fileURLToPath } from 'node:url';

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const PROTO = path.join(ROOT, 'docs/ui/prototypes');
const OUT = path.join(PROTO, '.harness');
fs.mkdirSync(OUT, { recursive: true });
const PORT = Number(process.env.HARNESS_PORT || 8199);

/* C66: board alignment across width bands (fixed icon slots / card reflow / no overflow). */
const BOARD = `goto('intercom');var rows=[...document.querySelectorAll('.board-row')];
  var hs=rows.map(r=>Math.round(r.getBoundingClientRect().height));
  var acts=rows.map(r=>Math.round(r.querySelector('.row-actions').getBoundingClientRect().height));
  var rights=rows.map(r=>Math.round(r.querySelector('.row-actions').getBoundingClientRect().right));
  var inside=rows.every(r=>{var rr=r.getBoundingClientRect();var mr=r.querySelector('[data-a=mon]').getBoundingClientRect();return mr.right<=rr.right-1;});
  var holder=rows.every(r=>r.querySelector('.floor-holder').getBoundingClientRect().width>20);
  var noOv=rows.every(r=>r.scrollWidth<=r.clientWidth);
  return {uniformH:new Set(hs).size===1,singleLine:new Set(acts).size===1&&acts[0]<=30,rightEdge1:new Set(rights).size===1,inside,holder,noOv};`;

const CASES = [
  ['floor', `goto('intercom');enterChannel('talk-delta');document.getElementById('chReq').click();
    var D=GROUPS.find(g=>g.id==='talk-delta');var idleGrant=D.state==='held'&&D.holder==='dispatch.x';
    enterChannel('talk-bravo');pttDown();var B=GROUPS.find(g=>g.id==='talk-bravo');var noJump=B.holder!=='dispatch.x';
    return {idleGrant,noJump};`],
  ['queue', `goto('intercom');var B=GROUPS.find(g=>g.id==='talk-bravo');var n0=B.q.length;
    for(var i=0;i<12;i++)qPush(B,{n:'x'+i,prio:false,you:false});var cap=B.q.length<=8;
    floorAction(GROUPS.indexOf(B),'prio');var rew=B.q.every(x=>x.prio===B.priority);return {cap,rew};`],
  ['replay', `goto('intercom');var A=GROUPS[0],B=GROUPS[1],C=GROUPS[2];
    enterChannel(A.id);recConsent[A.id]=true;recPush(A,'f',5);enterChannel(B.id,true);var scanKeeps=(recBuf[A.id]||[]).length===1;
    recConsent[B.id]=true;recPush(B,'f',4);enterChannel(C.id);var manualClears=(recBuf[B.id]||[]).length===0;
    recConsent[C.id]=true;recPush(C,'f',3);goto('monitor');var xpageClears=(recBuf[C.id]||[]).length===0;
    return {scanKeeps,manualClears,xpageClears};`],
  ['emergency', `goto('intercom');enterChannel('talk-alpha');var e=document.getElementById('chEmerg');e.click();
    var l1=document.getElementById('chEmerg').textContent;document.querySelector('.ch-tab[data-tab=rec]').click();document.querySelector('.ch-tab[data-tab=floor]').click();
    var l2=document.getElementById('chEmerg').textContent;var armPersist=l1==='Confirm insert?'&&l2==='Confirm insert?';
    document.getElementById('chEmerg').click();var A=GROUPS[0];var emergNoPress=pttHeld===false&&A.holder==='dispatch.x';
    return {armPersist,emergNoPress};`],
  ['tabs-size', `goto('intercom');enterChannel('talk-delta');document.querySelector('.ch-tab[data-tab=diag]').click();
    var m=id=>{var r=document.getElementById(id).getBoundingClientRect();return Math.round(r.width)+'x'+Math.round(r.height);};
    var diag=m('chVox')==='56x30'&&m('chCodec')==='56x30'&&m('chLock')==='56x30';
    document.querySelector('.ch-tab[data-tab=comms]').click();var tabSwitch=!!document.getElementById('chMsgSend');
    return {diag,tabSwitch};`],
  ['nav', `var icos=[...document.querySelectorAll('.nav-ico')];var hrefs=icos.map(x=>x.querySelector('use').getAttribute('href'));
    return {navSvg:icos.every(x=>!!x.querySelector('svg')),unique:new Set(hrefs).size===hrefs.length,countOk:hrefs.length===8};`],
  ['notif', `goto('monitor');applyProto('sim','extemerg');var c=document.getElementById('floorNotif');var xpage=!c.hidden;
    c.click();var clickLand=current==='intercom'&&chanOpen!==null;goto('monitor');applyProto('sim','extemerg');
    goto('intercom');var inpageHide=c.hidden;return {xpage,clickLand,inpageHide};`],
  ['groups', `goto('intercom');var gm=document.getElementById('grpManage');gm.focus();gm.click();
    document.getElementById('grpNewName').value='Harness crew';document.getElementById('grpCreate').click();
    var T=GROUPS.find(g=>g.name==='Harness crew');var created=!!T;
    var A=GROUPS[0];evConsent[A.id]=true;evPush(A,'f',5);enterChannel(A.id);
    document.querySelector('[data-gd="'+A.id+'"]').click();document.getElementById('cfmOk').click();
    var cascade=evBuf[A.id]===undefined&&chanOpen===null;
    document.getElementById('grpClose').click();var focusRestore=document.activeElement===gm;
    myRole='field';gm.focus();gm.click();document.getElementById('grpNewName').value='X';document.getElementById('grpCreate').click();
    var roleGuard=!GROUPS.some(g=>g.name==='X');myRole='dispatcher';
    enterChannel(GROUPS[1].id);gm.focus();gm.click();document.activeElement.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true,cancelable:true}));
    var escModal=chanOpen!==null&&!document.getElementById('grpScrim').classList.contains('show');
    return {created,cascade,focusRestore,roleGuard,escModal};`],
  ['evidence', `goto('intercom');var D=GROUPS.find(g=>g.id==='talk-delta');enterChannel(D.id);evConsent[D.id]=true;
    evBuf[D.id]=[{who:'a',dur:1,at:Date.now()-8*86400000,alert:'x',retain:7},{who:'b',dur:2,at:Date.now(),alert:'y',retain:7}];
    document.querySelector('.ch-tab[data-tab=rec]').click();
    var card=()=>[...document.querySelectorAll('.ch-card')].find(c=>c.textContent.includes('Event evidence'));
    var before=card().querySelectorAll('.replay-row').length;floorTick();var after=card().querySelectorAll('.replay-row').length;
    var purge=before===2&&after===1;
    document.getElementById('chEv').click();document.getElementById('cfmOk').click();var revoke=evBuf[D.id]===undefined;
    return {purge,revoke};`],
  ['admin-hub', `goto('admin');var cards=[...document.querySelectorAll('#adminHub .card')];var rows=[...document.querySelectorAll('#adminHub .board-row[data-go]')];
    var domains=cards.length===4&&rows.length===10;
    document.querySelector('#adminHub [data-go=devices]').click();
    var spoke=current==='devices';var parentActive=document.querySelector('.nav-item[data-nav=admin]').classList.contains('active');
    var crumb=document.getElementById('crumb').textContent.indexOf('Admin /')===0;
    return {domains,spoke,parentActive,crumb};`],
  ['device-drawer', `goto('devices');var row=document.querySelector('#devBody .dev-primary');row.click();
    var open=document.getElementById('devDrawer').classList.contains('open');
    var hiddenClosed=document.getElementById('devDrawer').getAttribute('aria-hidden')==='false';
    var initialFocus=document.activeElement.id==='ddClose';
    var tabs=document.querySelectorAll('.dd-tab').length===6;
    var t0=document.querySelector('.dd-tab[aria-selected="true"]');t0.focus();
    t0.dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowRight',bubbles:true,cancelable:true}));
    var roving=document.querySelector('.dd-tab[aria-selected="true"]').dataset.dt==='config';
    var els=[...document.querySelectorAll('#devDrawer button,#devDrawer input,#devDrawer select')].filter(x=>!x.disabled);
    els[els.length-1].focus();els[els.length-1].dispatchEvent(new KeyboardEvent('keydown',{key:'Tab',bubbles:true,cancelable:true}));
    var trap=document.activeElement===els[0];
    confirmDlg('probe','probe',function(){});
    document.body.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true,cancelable:true}));
    var modalFirst=!document.querySelector('.modal-scrim.show')&&document.getElementById('devDrawer').classList.contains('open');
    document.body.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true,cancelable:true}));
    var esc=!document.getElementById('devDrawer').classList.contains('open');
    var restored=document.activeElement.classList.contains('dev-primary')||document.activeElement.id==='devSearch';
    var list=document.querySelectorAll('#devBody tr').length>0;
    var closedInert=getComputedStyle(document.getElementById('devDrawer')).visibility==='hidden';
    var ddIcon=!!document.querySelector('#ddClose use[href="#i-chevron"]')&&document.getElementById('ddClose').classList.contains('region-collapse');
    return {open,hiddenClosed,initialFocus,tabs,roving,trap,modalFirst,esc,restored,list,closedInert,ddIcon};`],
  ['device-drawer-narrow', `goto('devices');document.querySelector('#devBody .dev-primary').click();
    var w=Math.round(document.getElementById('devDrawer').getBoundingClientRect().width);
    return {fullWidth:w>=innerWidth-2};`, 600],
  ['device-config', `applyProto('sim','vision');goto('devices');var row=document.querySelector('#devBody .dev-primary');row.click();
    document.querySelector('.dd-tab[data-dt=config]').click();var D=DEVICES.find(x=>x.id===row.dataset.id);
    var inp=document.querySelector('[data-dk="video.fps"]');inp.value='30';inp.dispatchEvent(new Event('input',{bubbles:true}));
    var diff=document.querySelectorAll('#ddDiff .diff-row').length===1;
    document.getElementById('ddApply').click();
    var applied=(D.overrides['video.fps']||{}).state==='applied'&&AUDIT[0].action==='device.config.apply';
    document.getElementById('ddUndo').click();
    var rb=(D.overrides['video.fps']||{}).val==='25'&&AUDIT[0].action==='device.config.rollback';
    var OFF=DEVICES.find(x=>x.status==='offline'&&!x.revoked);openDevDrawer(OFF.id);document.querySelector('.dd-tab[data-dt=config]').click();
    var i2=document.querySelector('[data-dk="video.fps"]');i2.value='15';i2.dispatchEvent(new Event('input',{bubbles:true}));document.getElementById('ddApply').click();
    var staged=(OFF.overrides['video.fps']||{}).state==='staged';
    applyProto('sim','reconnect');
    var flush=(OFF.overrides['video.fps']||{}).state==='applied'&&AUDIT[0].action==='device.config.delivered';
    var DEG=DEVICES.find(x=>x.status==='degraded');openDevDrawer(DEG.id);document.querySelector('.dd-tab[data-dt=config]').click();
    var i3=document.querySelector('[data-dk="video.fps"]');i3.value='20';i3.dispatchEvent(new Event('input',{bubbles:true}));document.getElementById('ddApply').click();
    var failed=(DEG.overrides['video.fps']||{}).state==='failed'&&!!document.querySelector('[data-retry]');
    document.querySelector('[data-retry]').click();
    var retry=(DEG.overrides['video.fps']||{}).state==='failed';
    return {diff,applied,rb,staged,flush,failed,retry};`],
  ['device-role', `applyProto('sim','vision');goto('devices');var M={};
    ['viewer','dispatcher','admin','field'].forEach(r=>{myRole=r;renderDevices();openDevDrawer('CAM-05');
      var g=s=>{var e=document.querySelector(s);return e?(e.getAttribute('aria-disabled')==='true'):null;};
      document.querySelector('.dd-tab[data-dt=config]').click();var cfg=g('[data-dk]');
      document.querySelector('.dd-tab[data-dt=identity]').click();var cap=g('[data-caprm]');var save=g('#ddSave');
      closeDevDrawer();var ap=g('.dev-approve');var rj=g('.dev-reject');
      M[r]={cfg:cfg,cap:cap,save:save,approve:ap,reject:rj};});
    var viewerOk=M.viewer.cfg===true&&M.viewer.cap===true&&M.viewer.save===true&&M.viewer.approve===true&&M.viewer.reject===true;
    var fieldOk=M.field.cfg===true&&M.field.approve===true;
    var dispOk=M.dispatcher.cfg===false&&M.dispatcher.cap===true&&M.dispatcher.save===false&&M.dispatcher.approve===true;
    var adminOk=M.admin.cfg===false&&M.admin.cap===false&&M.admin.save===false&&M.admin.approve===false&&M.admin.reject===false;
    myRole='viewer';applyProto('role','admin');openDevDrawer('CAM-01');var repaint=document.getElementById('ddRole').textContent==='admin';closeDevDrawer();
    myRole='dispatcher';renderDevices();
    return {viewerOk,fieldOk,dispOk,adminOk,repaint};`],
  ['accounts', `myRole='admin';syncUserchip();goto('admin');
    var mrows=document.querySelectorAll('#adminHub .tbl tbody tr').length;
    var matrixRows9=mrows===9;
    var states=[...document.querySelectorAll('#adminHub .cap-state')].map(x=>x.textContent);
    var statesDerive=states.filter(t=>t==='active').length===7&&states.filter(t=>t==='absent').length===2;
    goto('accounts');document.getElementById('accNew').click();
    var focusInitial=document.activeElement.id==='accUser';
    document.getElementById('accUser').value='test1';document.getElementById('accPass').value='pw';
    document.getElementById('accRoleBtn').click();document.querySelector('#accRolePop [data-ro=operator]').click();
    document.querySelector('#accScope [data-sc=VH-07]').click();
    var nonVehGated=document.querySelector('#accScope [data-sc=CAM-01]').getAttribute('aria-disabled')==='true';
    document.getElementById('accSave').click();
    var created=ACCOUNTS.some(a=>a.u==='test1'&&a.role==='operator'&&a.vehicles.includes('VH-07'));
    document.querySelector('.acc-edit[data-u=ops1]').click();document.getElementById('accRoleBtn').click();document.querySelector('#accRolePop [data-ro=dispatcher]').click();document.getElementById('accSave').click();
    var edited=ACCOUNTS.find(a=>a.u==='ops1').role==='dispatcher';
    var focusRestore=document.activeElement.classList.contains('acc-edit');
    document.querySelector('.acc-del[data-u=view1]').click();document.getElementById('cfmOk').click();
    var deleted=!ACCOUNTS.some(a=>a.u==='view1');
    var selfGuard=document.querySelector('.acc-del[data-u="dispatch.x"]').getAttribute('aria-disabled')==='true';
    var guardKeyboard=!document.querySelector('.acc-del[data-u="dispatch.x"]').disabled;
    myRole='dispatcher';renderAccounts();var dispatcherRead=document.querySelectorAll('#accBody tr').length>0&&document.getElementById('accNew').getAttribute('aria-disabled')==='true';
    myRole='viewer';renderAccounts();var viewerNoAccess=document.getElementById('accBody').textContent.includes('No access');
    myRole='operator';renderAccounts();var operatorNoAccess=document.getElementById('accBody').textContent.includes('No access');
    myRole='admin';applyProto('cap','provider:accessbase');renderAccounts();
    var delegatedGuard=document.getElementById('accNew').getAttribute('aria-disabled')==='true';
    goto('admin');var delRow=[...document.querySelectorAll('#adminHub .tbl tbody tr')].find(r=>r.textContent.includes('Accounts'));
    var delAff=!!delRow.querySelector('a[data-ext]')&&!!delRow.querySelector('[data-force]');
    delRow.querySelector('[data-force]').click();document.getElementById('cfmOk').click();
    var forced=capState('accounts')==='degraded';
    goto('accounts');var breakGlass=!!document.getElementById('accBgNew');
    document.getElementById('accBgNew').click();document.getElementById('cfmOk').click();
    var bgAudited=AUDIT[0].action==='account.breakglass';
    document.getElementById('accBgReset').click();document.getElementById('cfmOk').click();
    var bgReset=AUDIT[0].action==='account.breakglass.reset';
    applyProto('cap','health:unreachable');goto('admin');
    var degradedRow=[...document.querySelectorAll('#adminHub .cap-state')].some(x=>x.textContent==='degraded');
    var banner=!document.getElementById('degBanner').hidden;
    applyProto('cap','provider:builtin');applyProto('cap','health:reachable');
    applyProto('sim','accerror');var errState=document.getElementById('accErr').textContent.includes('unreachable');
    document.getElementById('accRetry').click();var loadingState=document.getElementById('accBody').textContent.includes('Loading');
    accError=false;accLoading=false;accQ='zzz';renderAccounts();var emptyState=document.getElementById('accBody').textContent.includes('No accounts match');accQ='';renderAccounts();
    var lightDefault=document.documentElement.dataset.theme==='light';
    return {matrixRows9,statesDerive,focusInitial,created,nonVehGated,edited,focusRestore,deleted,selfGuard,guardKeyboard,dispatcherRead,viewerNoAccess,operatorNoAccess,delegatedGuard,delAff,forced,breakGlass,bgAudited,bgReset,degradedRow,banner,errState,loadingState,emptyState,lightDefault};`],
  ['psk', [
    `myRole='admin';goto('keys');
     document.getElementById('pskView').click();
     var viewAudited=pskRevealed&&AUDIT[0].action==='psk.view'&&document.getElementById('pskVal').textContent===PSK.value;
     document.getElementById('pskView').click();
     document.getElementById('pskRotate').click();
     var rotateFocus=document.activeElement.id==='pskNew'&&document.getElementById('pskScrim').classList.contains('show');
     document.getElementById('pskNew').value='abc';document.getElementById('pskOk').click();
     var rotateValidate=PSK.value.indexOf('custom')<0&&!document.getElementById('secretScrim').classList.contains('show');
     document.getElementById('pskRotate').click();document.getElementById('pskNew').value='ms-psk-custom-123';document.getElementById('pskOk').click();
     var loadingState=document.getElementById('keyBody').textContent.includes('Loading');
     return {viewAudited,rotateFocus,rotateValidate,loadingState};`,
    `var onceDisplay=document.getElementById('secretScrim').classList.contains('show')&&document.getElementById('secVal').textContent==='ms-psk-custom-123';
     document.getElementById('secDone').click();
     var closeGuard=document.getElementById('cfmScrim').classList.contains('show')&&document.getElementById('secretScrim').classList.contains('show');
     document.getElementById('cfmCancel').click();
     var closeGuardKeeps=document.getElementById('secretScrim').classList.contains('show');
     document.getElementById('secCopy').click();
     var copyHandled=secretCopied||document.getElementById('secCopyHint').textContent.length>0;
     secretCopied=true;document.getElementById('secDone').click();
     var safeClose=!document.getElementById('secretScrim').classList.contains('show');
     var rotated=PSK.value==='ms-psk-custom-123'&&AUDIT.some(a=>a.action==='psk.rotate');
     return {onceDisplay,closeGuard,closeGuardKeeps,copyHandled,safeClose,rotated};`,
    `document.getElementById('newKey').click();
     var createFocus=document.activeElement.id==='keyId';
     document.getElementById('keyId').value='key-t1';document.getElementById('keyLabel').value='test key';
     document.getElementById('keyRoleBtn').click();document.querySelector('#keyRolePop [data-kro=operator]').click();
     document.querySelector('#keyScope [data-ksc=VH-07]').click();
     var nonVehGated=document.querySelector('#keyScope [data-ksc=CAM-01]').getAttribute('aria-disabled')==='true';
     document.getElementById('keyOk').click();
     var createLoading=document.getElementById('keyBody').textContent.includes('Loading');
     return {createFocus,nonVehGated,createLoading};`,
    `var keySecret=document.getElementById('secretScrim').classList.contains('show')&&document.getElementById('secVal').textContent.indexOf('msk_')===0;
     secretCopied=true;document.getElementById('secDone').click();
     var created=KEYS.some(k=>k.id==='key-t1'&&k.role==='operator'&&k.vehicles.includes('VH-07'))&&AUDIT.some(a=>a.action==='key.create');
     document.querySelector('.key-revoke[data-id=key-t1]').click();
     var revokeConfirm=document.getElementById('cfmScrim').classList.contains('show');
     document.getElementById('cfmOk').click();
     var revoked=!KEYS.some(k=>k.id==='key-t1')&&AUDIT[0].action==='key.revoke';
     applyProto('cap','provider:accessbase');renderKeys();
     var delegatedGuard=document.getElementById('newKey').getAttribute('aria-disabled')==='true'&&document.getElementById('pskRotate').getAttribute('aria-disabled')==='true';
     applyProto('cap','health:unreachable');renderKeys();
     var bgPsk=!!document.getElementById('pskBg');
     document.getElementById('pskBg').click();document.getElementById('cfmOk').click();
     return {keySecret,created,revokeConfirm,revoked,delegatedGuard,bgPsk};`,
    `var bgAudited=AUDIT.some(a=>a.action==='psk.breakglass');
     applyProto('cap','provider:builtin');applyProto('cap','health:reachable');
     applyProto('sim','keyerror');var errState=document.getElementById('keyErr').textContent.includes('unreachable');
     document.getElementById('keyRetry').click();var retryLoading=document.getElementById('keyBody').textContent.includes('Loading');
     keyError=false;keyLoading=false;keyQ='zzz';renderKeys();var emptyState=document.getElementById('keyBody').textContent.includes('No keys match');keyQ='';renderKeys();
     return {bgAudited,errState,retryLoading,emptyState};`
  ]],
  ['creds', [
    `myRole='admin';goto('devices');var P=DEVICES.find(x=>x.status==='pending');openDevDrawer(P.id);
     document.querySelector('.dd-tab[data-dt=creds]').click();
     var visionGated=document.getElementById('credReg').getAttribute('aria-disabled')==='true';
     var emptyState=document.getElementById('ddBody').textContent.includes('no credential yet');
     applyProto('sim','vision');
     var simulateLeg=document.getElementById('credReg').getAttribute('aria-disabled')!=='true';
     document.getElementById('credReg').click();
     var focusInitial=document.activeElement.id==='credName';
     document.getElementById('credName').value='renamed-cam';document.getElementById('credSecret').value='ms-dev-provided-1';
     document.getElementById('credOk').click();
     var loadingState=document.getElementById('ddBody').textContent.includes('Resetting')||credLoading;
     return {visionGated,emptyState,simulateLeg,focusInitial,loadingState};`,
    `var registerOnce=document.getElementById('secretScrim').classList.contains('show')&&document.getElementById('secVal').textContent==='ms-dev-provided-1';
     document.getElementById('secDone').click();
     var closeGuard=document.getElementById('cfmScrim').classList.contains('show');
     document.getElementById('cfmCancel').click();secretCopied=true;document.getElementById('secDone').click();
     var P=DEVICES.find(x=>x.id===devOpen);
     var registered=P.status==='online'&&P.name==='renamed-cam'&&AUDIT.some(a=>a.action==='device.register')&&AUDIT.some(a=>a.action==='device.approve');
     return {registerOnce,closeGuard,registered};`,
    `document.getElementById('credReset').click();
     var resetConfirm=document.getElementById('cfmScrim').classList.contains('show');
     document.getElementById('cfmOk').click();
     var resetLoading=credLoading;
     return {resetConfirm,resetLoading};`,
    `var resetOnce=document.getElementById('secretScrim').classList.contains('show')&&AUDIT.some(a=>a.action==='device.secret.reset');
     secretCopied=true;document.getElementById('secDone').click();
     var revokeSemantics=document.getElementById('ddBody').textContent.includes('DELETE');
     var rg=document.getElementById('credReg');rg.focus();rg.click();
     document.getElementById('credCancel').click();
     var focusRestore=document.activeElement.id==='credReg';
     var rg2=document.getElementById('credReg');rg2.focus();rg2.click();
     document.getElementById('credName').dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true,cancelable:true}));
     var escOrder=!document.getElementById('credScrim').classList.contains('show');
     myRole='viewer';renderDrawer();
     var guardKeyboard=!document.getElementById('credReset').disabled&&document.getElementById('credReset').getAttribute('aria-disabled')==='true';
     myRole='admin';applyProto('sim','crederror');
     var errState=document.getElementById('ddBody').textContent.includes('unreachable');
     document.getElementById('credRetry').click();
     var retryLoading=credLoading;
     credError=false;credLoading=false;renderDrawer();
     return {resetOnce,revokeSemantics,focusRestore,escOrder,guardKeyboard,errState,retryLoading};`
  ]],
  ['fleet', `myRole='admin';goto('devices');
    document.querySelector('#fleetLens [data-lens=fleet]').click();
    var rows=[...document.querySelectorAll('#devBody tr')];
    var lensFilter=rows.length===3&&rows.every(r=>r.textContent.includes('VH-'));
    var lensAria=document.getElementById('fleetLens').getAttribute('role')==='radiogroup'&&document.querySelector('#fleetLens [data-lens=fleet]').getAttribute('aria-checked')==='true'&&document.querySelector('#fleetLens [data-lens=all]').tabIndex===-1;
    var strip=document.getElementById('fleetStrip').textContent;
    var healthStrip=strip.includes('3')&&strip.includes('Signal lost')&&strip.includes('Agent unreachable')&&strip.includes('Stalled topics')&&strip.includes('Down processes');
    var rowSummary=document.getElementById('devBody').textContent.includes('up 12h');
    goto('overview');document.getElementById('ovFleetOpen').click();
    var deepLink=current==='devices'&&devLens==='fleet';
    openDevDrawer('VH-07');document.querySelector('.dd-tab[data-dt=telemetry]').click();
    var t7=document.getElementById('ddBody').textContent;
    var telemetrySignalUptime=t7.includes('ok')&&t7.includes('12h')&&t7.includes('1920x1080');
    openDevDrawer('VH-02');document.querySelector('.dd-tab[data-dt=telemetry]').click();
    var t2=document.getElementById('ddBody').textContent;
    var procThreeState=t2.includes('down')&&t2.includes('lost');
    var topicStalled=t2.includes('stalled');
    openDevDrawer('VH-11');document.querySelector('.dd-tab[data-dt=telemetry]').click();
    var unreachable=document.getElementById('ddBody').textContent.includes('Agent unreachable');
    closeDevDrawer();goto('accounts');document.getElementById('accNew').click();
    var scopeIds=[...document.querySelectorAll('#accScope [data-sc]')].map(x=>x.dataset.sc).filter(id=>id.startsWith('VH-'));
    var scopeCrossLink=scopeIds.length===3;
    document.getElementById('accCancel').click();
    applyProto('sim','fleeterror');
    var errState=document.getElementById('fleetStrip').textContent.includes('unreachable');
    document.getElementById('fleetRetry').click();
    var loadingState=fleetLoading;
    fleetError=false;fleetLoading=false;
    document.querySelector('#devSeg [data-f=pending]').click();
    var emptyState=document.getElementById('devBody').textContent.includes('No devices match');
    document.querySelector('#devSeg [data-f=all]').click();
    return {lensFilter,lensAria,healthStrip,rowSummary,deepLink,telemetrySignalUptime,procThreeState,topicStalled,unreachable,scopeCrossLink,errState,loadingState,emptyState};`],
  ['monitor-link', `goto('monitor');openPicker();
    var treeRole=document.getElementById('pickAvail').getAttribute('role')==='tree';
    var tw=document.querySelector('#pickAvail button.tw');tw.focus();
    var twistyFocusable=tw.tabIndex===0;
    tw.dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowDown',bubbles:true,cancelable:true}));
    var treeKeyboard=document.activeElement!==tw;
    var tw2=document.querySelector('#pickAvail button.tw');tw2.focus();
    tw2.dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowRight',bubbles:true,cancelable:true}));
    var treeExpand=document.querySelector('#pickAvail .pick-dev').getAttribute('aria-expanded')==='true';
    document.getElementById('playAll').click();
    var playAllGuard=pickSel.length>0&&pickSel.length<=16&&!pickSel.some(id=>id.startsWith('vh-02'));
    pickSel=Array.from({length:16},(_,i)=>'pad-'+i).concat(['extra-17']);renderPick();
    var cap16Guard17th=pickSel.length===17&&document.getElementById('pickApply').classList.contains('is-disabled');
    pickSel=Array.from({length:16},(_,i)=>'pad-'+i);renderPick();
    var bulk=[...document.querySelectorAll('#pickAvail .pick-dev input')].find(c=>!c.checked);
    if(bulk){bulk.checked=true;bulk.onchange();}
    var bulkGuard=bulk?pickSel.length===16&&bulk.classList.contains('is-disabled'):false;
    pickSel=connectedStreamIds().slice(0,4);renderPick();
    document.getElementById('pickApply').click();
    document.getElementById('statsToggle').click();
    var perStreamMetrics=document.querySelector('#tiles .tile-stats').textContent.includes('FPS');
    var honestBanner=document.getElementById('monSimNote').textContent.includes('Simulated video');
    applyContent('empty');var emptyState=document.body.dataset.content==='empty';
    applyContent('loading');var loadingState=getComputedStyle(document.getElementById('wallLoading')).display!=='none';
    applyContent('error');var errState=document.body.dataset.content==='error';
    document.getElementById('liveRetry').click();var retryWorks=document.body.dataset.content==='data';
    applyContent('data');
    return {treeRole,twistyFocusable,treeKeyboard,treeExpand,playAllGuard,cap16Guard17th,bulkGuard,perStreamMetrics,honestBanner,emptyState,loadingState,errState,retryWorks};`],
  ['audit-ring', `myRole='admin';goto('audit');
    var acts=[...document.querySelectorAll('#auditBody tr td:nth-child(3)')].map(x=>x.textContent);
    var ringRows=acts.length>0&&acts.every(a=>/^(auth|account|psk|key|device|access)\./.test(a));
    var meta=document.getElementById('auditMeta').textContent;
    var retentionLabel=meta.includes('Retention 90 d')&&meta.includes('simulated');
    var hl=document.getElementById('auditHistory');
    var historyLink=hl.getAttribute('aria-disabled')==='true'&&(hl.dataset.reason||'').includes('/talk/history');
    var srcs=[...document.querySelectorAll('#auditBody tr')].map(r=>[r.children[2].textContent,r.children[4].textContent]);
    var loginQuerySimulated=srcs.some(x=>x[0]==='auth.login'&&x[1]==='simulated');
    var logoutSimulated=srcs.some(x=>x[0]==='auth.logout'&&x[1]==='simulated');
    document.querySelector('#auditSeg [data-ak=identity]').click();
    var kindFilter=[...document.querySelectorAll('#auditBody tr td:nth-child(3)')].every(x=>/^(auth|account)\./.test(x.textContent));
    AUDIT.unshift({t:'12:11:00',actor:'admin.x',action:'account.breakglass',target:'emg-1',ok:true,source:'core'});
    document.querySelector('#auditSeg [data-ak=bg]').click();
    var bgRing=[...document.querySelectorAll('#auditBody tr td:nth-child(3)')].some(x=>x.textContent==='account.breakglass');
    document.querySelector('#auditSeg [data-ak=all]').click();
    applyProto('cap','provider:accessbase');renderAudit();
    var sourceProvider=[...document.querySelectorAll('#auditBody tr')].some(r=>r.children[4].textContent==='provider');
    applyProto('cap','provider:builtin');renderAudit();
    auditQ='zzz';renderAudit();var emptyState=document.getElementById('auditBody').textContent.includes('No security events match');auditQ='';renderAudit();
    applyProto('sim','auditerror');var errState=document.getElementById('auditErr').textContent.includes('unreachable');
    document.getElementById('auditRetry').click();var loadingState=auditLoading;
    auditError=false;auditLoading=false;renderAudit();
    return {ringRows,retentionLabel,historyLink,loginQuerySimulated,logoutSimulated,kindFilter,bgRing,sourceProvider,emptyState,errState,loadingState};`],
  ['nav-collapse', `var b=document.getElementById('navCollapse');
    var initSvg=!!b.querySelector('svg use[href="#i-chevron-left"]');
    b.click();
    var sv=b.querySelector('svg');var r=sv.getBoundingClientRect();
    var collapsedIcon=!!b.querySelector('svg use[href="#i-chevron"]')&&r.width<=28&&r.height<=28;
    var collapsedState=document.getElementById('body').classList.contains('nav-min');
    b.click();
    var expandedIcon=!!b.querySelector('svg use[href="#i-chevron-left"]')&&!document.getElementById('body').classList.contains('nav-min');
    applyProto('view','rail');
    var railIcon=!!b.querySelector('svg use[href="#i-chevron"]');
    applyProto('view','desktop');
    var resetIcon=!!b.querySelector('svg use[href="#i-chevron-left"]');
    var aria=b.getAttribute('aria-label')==='Collapse navigation';
    return {initSvg,collapsedIcon,collapsedState,expandedIcon,railIcon,resetIcon,aria};`],
  ['meet-lobby', `goto('meeting');
    var cards=document.querySelectorAll('#meetGrid .card').length===2;
    var typeChips=[...document.querySelectorAll('#meetGrid .chip')].map(x=>x.textContent);
    var typeOk=typeChips.includes('Video')&&typeChips.includes('Voice');
    goto('rooms');var noObject=!document.getElementById('roomBody').textContent.includes('[object Object]');goto('meeting');
    document.getElementById('newMeet').click();
    var createForm=document.getElementById('meetScrim').classList.contains('show')&&document.activeElement.id==='meetName';
    var seg=document.getElementById('meetTypeSeg');
    var videoRadio=seg.getAttribute('role')==='radiogroup'&&seg.querySelector('[data-mt=video]').getAttribute('aria-checked')==='true';
    seg.querySelector('[data-mt=video]').focus();
    seg.querySelector('[data-mt=video]').dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowLeft',bubbles:true,cancelable:true}));
    var roving=seg.querySelector('[data-mt=voice]').getAttribute('aria-checked')==='true'&&seg.querySelector('[data-mt=voice]').tabIndex===0;
    document.getElementById('meetName').value='Test meet';
    document.getElementById('meetOk').click();
    var created=ROOMS.some(r=>r.name==='Test meet'&&r.video===false&&r.host===CUR_USER&&r.state==='idle')&&AUDIT[0].action==='meet.create';
    var focusBack=document.activeElement.id==='newMeet';
    document.querySelector('.meet-invite').click();
    var inviteHint=document.getElementById('meetCopyHint').textContent.length>0;
    var j0=document.querySelector('.meet-join');j0.focus();j0.click();
    var joinOpensPrejoin=document.getElementById('prejoinScrim').classList.contains('show');
    document.getElementById('pjCancel').click();
    var RV=ROOMS.find(r=>r.id==='meet-review');RV.host=CUR_USER;renderMeeting();
    var reopenBtn=!!document.querySelector('.meet-reopen[data-id=meet-review]');
    document.querySelector('.meet-reopen[data-id=meet-review]').click();
    var reopened=RV.state==='idle'&&RV.endReason===null&&AUDIT[0].action==='meet.reopen';
    return {cards,typeOk,noObject,createForm,videoRadio,roving,created,focusBack,inviteHint,joinOpensPrejoin,reopenBtn,reopened};`],
  ['meet-prejoin', [
    `goto('meeting');
     document.getElementById('newMeet').click();document.getElementById('meetName').value='Voice test';
     document.querySelector('#meetTypeSeg [data-mt=voice]').click();document.getElementById('meetOk').click();
     var VT=ROOMS.find(r=>r.name==='Voice test');
     var jb=document.querySelector('.meet-join[data-id="'+VT.id+'"]');jb.focus();jb.click();
     var nameFocus=document.activeElement.id==='pjName';
     var voiceCamGuard=document.getElementById('pjCamPreview').textContent.includes('no camera')&&document.getElementById('pjJoin').getAttribute('aria-disabled')!=='true';
     document.getElementById('pjCancel').click();
     var cancelRestores=document.activeElement.classList.contains('meet-join');
     document.querySelector('.meet-join[data-id="'+VT.id+'"]').click();
     document.getElementById('pjName').dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true,cancelable:true}));
     var escCancel=!document.getElementById('prejoinScrim').classList.contains('show');
     var ST=ROOMS.find(r=>r.id==='meet-standup');ST.locked=true;renderMeeting();
     var lockedGuard=document.querySelector('.meet-join[data-id=meet-standup]').getAttribute('aria-disabled')==='true';
     openPrejoin('meet-standup');
     var lockedJoinGuard=document.getElementById('pjJoin').getAttribute('aria-disabled')==='true';
     document.getElementById('pjCancel').click();ST.locked=false;renderMeeting();
     return {nameFocus,voiceCamGuard,cancelRestores,escCancel,lockedGuard,lockedJoinGuard};`,
    `openPrejoin('meet-standup');document.getElementById('pjJoin').click();
     var connectingShellKept=meetPhase==='connecting'&&getComputedStyle(document.getElementById('sidebar')).display!=='none';
     return {connectingShellKept};`,
    `var hostLive=meetPhase==='live'&&meetLive==='meet-standup'&&AUDIT.some(a=>a.action==='meet.join');
     meetLeave();var ST=ROOMS.find(r=>r.id==='meet-standup');ST.host='admin.x';
     openPrejoin('meet-standup');document.getElementById('pjJoin').click();
     return {hostLive};`,
    `var waitingGuestScreen=meetPhase==='waiting'&&document.getElementById('meetPhaseCard').textContent.includes('Waiting for host');
     admitWaiting('meet-standup',CUR_USER);
     var guestAdmit=meetPhase==='live'&&AUDIT.some(a=>a.action==='meet.admit');
     meetLeave();openPrejoin('meet-standup');document.getElementById('pjJoin').click();
     return {waitingGuestScreen,guestAdmit};`,
    `denyWaiting('meet-standup',CUR_USER);
     var denyFeedback=meetPhase==='lobby'&&AUDIT.some(a=>a.action==='meet.deny');
     var ST=ROOMS.find(r=>r.id==='meet-standup');ST.host='dispatch.x';renderMeeting();
     return {denyFeedback};`
  ]],
  ['meet-live', `goto('meeting');meetLive='meet-standup';meetPhase='live';renderMeetPhase();
    var shellHidden=getComputedStyle(document.getElementById('sidebar')).display==='none'&&getComputedStyle(document.getElementById('topbar')).display==='none';
    var tilesAdaptive=document.getElementById('meetTiles').className==='g3';
    var av=[...document.querySelectorAll('.mtile')].find(t=>!t.querySelector('.tile-scan'));
    var avatarNotBlack=!!av&&getComputedStyle(av).backgroundColor!=='rgb(0, 0, 0)';
    var t0=document.querySelector('.mtile');var pn0=t0.dataset.pn;var bg0=getComputedStyle(t0).backgroundColor;
    speakerTick();
    var t1=document.querySelector('.mtile[data-pn="'+pn0+'"]');
    var speakingBorderFillUnchanged=!!t1&&getComputedStyle(t1).backgroundColor===bg0&&!!document.querySelector('.mtile.speaking');
    var pip=document.getElementById('meetPip');var pr=pip.getBoundingClientRect();var cr=document.getElementById('meetCanvas').getBoundingClientRect();
    var selfPipMirroredWidthPos=getComputedStyle(pip).transform.includes('-1')&&Math.abs(pr.width-cr.width/6)<3&&pr.bottom<cr.bottom-50;
    toast('probe');
    var muteChipIndependentOfToast=document.getElementById('muteChip').closest('#toast')===null&&getComputedStyle(document.getElementById('muteChip')).display!=='none';
    var R=meetRoom();while(R.parts.length<10)R.parts.push({n:'x'+R.parts.length,host:false,muted:true,cam:false,hand:false});
    meetPage=1;renderMeetTiles();
    var pager=document.getElementById('meetPager').textContent.includes('Page 2 / 2');
    document.getElementById('mkLayout').click();
    var layoutSwitchKeepsState=meetPage===1&&meetLayout==='speaker'&&document.getElementById('meetTiles').className==='speaker'&&!!document.querySelector('#meetTiles .mtile.main');
    document.getElementById('mkLayout').click();
    var estopSlotEmpty=document.querySelector('#meetCanvas [data-estop]')===null;
    document.body.classList.add('meet-hideov');
    var hideExemptions=getComputedStyle(document.getElementById('muteChip')).display!=='none'&&getComputedStyle(document.getElementById('mkLeave')).display!=='none'&&getComputedStyle(document.getElementById('meetSimChip')).display!=='none'&&getComputedStyle(document.querySelector('.mtile .osd')).display==='none';
    document.body.classList.remove('meet-hideov');
    var coarseRule=[...document.querySelectorAll('style')].some(st=>st.textContent.includes('(pointer:coarse){#meetBar .btn{height:44px'));
    holdStart('leave');for(var i=0;i<20;i++)holdTick();
    var leaveHold1s=meetPhase==='lobby';
    meetLive='meet-standup';meetPhase='live';renderMeetPhase();
    escHoldStart();for(var j=0;j<20;j++)escHoldTick();
    var escHold1s=meetPhase==='lobby';
    meetLive='meet-standup';meetPhase='live';renderMeetPhase();
    document.body.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true,cancelable:true}));
    document.body.dispatchEvent(new KeyboardEvent('keyup',{key:'Escape',bubbles:true,cancelable:true}));
    var shortEscHint=getComputedStyle(document.getElementById('escHint')).display==='block'&&meetPhase==='live';
    meetLive='meet-review';meetPhase='live';renderMeetPhase();
    var voiceCamGuardLive=document.getElementById('mkCam').getAttribute('aria-disabled')==='true';
    meetLeave();
    return {shellHidden,tilesAdaptive,avatarNotBlack,speakingBorderFillUnchanged,selfPipMirroredWidthPos,muteChipIndependentOfToast,pager,layoutSwitchKeepsState,estopSlotEmpty,hideExemptions,coarseRule,leaveHold1s,escHold1s,shortEscHint,voiceCamGuardLive};`],
  ['meet-host', `goto('meeting');meetLive='meet-standup';meetPhase='live';renderMeetPhase();
    var R=meetRoom();
    var mr=document.getElementById('mkRoster');mr.focus();mr.click();
    var panelOpen=document.getElementById('meetPanel').classList.contains('open');
    var hostBadge=document.getElementById('mpBody').textContent.includes('host');
    R.waiting.push({n:'guest1',at:Date.now()});renderRoster();syncMpBadge();
    var badge=document.getElementById('mpBadge').hidden===false&&document.getElementById('mpBadge').textContent==='1';
    document.querySelector('.mp-admit[data-n=guest1]').click();
    var waitingAdmit=R.parts.some(p=>p.n==='guest1')&&R.waiting.length===0&&AUDIT.some(a=>a.action==='meet.admit');
    R.waiting.push({n:'guest2',at:Date.now()});renderRoster();
    document.querySelector('.mp-deny[data-n=guest2]').click();
    var waitingDeny=R.waiting.length===0&&AUDIT.some(a=>a.action==='meet.deny');
    document.getElementById('mpMuteAll').click();
    var muteAllExcludesSelf=R.parts.filter(p=>p.n!==CUR_USER).every(p=>p.muted)&&!R.parts.find(p=>p.n===CUR_USER).muted;
    document.getElementById('mpLock').click();
    var lockGuardsJoin=R.locked===true;renderMeeting();
    var lobbyLocked=document.querySelector('.meet-join[data-id=meet-standup]').getAttribute('aria-disabled')==='true';
    document.getElementById('mpLock').click();
    var unlockOk=R.locked===false;
    document.querySelector('.mp-remove[data-n=guest1]').click();
    var removeConfirm=document.getElementById('cfmScrim').classList.contains('show');
    document.getElementById('cfmOk').click();
    var removed=!R.parts.some(p=>p.n==='guest1')&&AUDIT.some(a=>a.action==='meet.remove');
    var noSelfRemove=!document.querySelector('.mp-remove[data-n="'+CUR_USER+'"]');
    R.host='admin.x';renderRoster();
    var nonHostGuard=document.getElementById('mpMuteAll').getAttribute('aria-disabled')==='true';
    R.host='dispatch.x';renderRoster();
    document.getElementById('mpTabChat').click();
    var chatEmptyState=document.getElementById('mpChatList').textContent.includes('No messages yet');
    document.getElementById('mpChatSend').click();
    var chatEmptyInputGuard=R.chat.length===0;
    document.getElementById('mpChatIn').value='hello ops';document.getElementById('mpChatSend').click();
    var chatSend=R.chat.some(m=>m.txt==='hello ops'&&m.n===CUR_USER);
    var handBtn=document.getElementById('mkHand');handBtn.click();
    var handRaise=R.parts.find(p=>p.n===CUR_USER).hand===true;
    handBtn.click();
    var handLower=R.parts.find(p=>p.n===CUR_USER).hand===false;
    document.body.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true,cancelable:true}));
    var panelEscKeepsMeeting=!document.getElementById('meetPanel').classList.contains('open')&&meetPhase==='live';
    meetLeave();
    return {panelOpen,hostBadge,badge,waitingAdmit,waitingDeny,muteAllExcludesSelf,lockGuardsJoin,lobbyLocked,unlockOk,removeConfirm,removed,noSelfRemove,nonHostGuard,chatEmptyState,chatEmptyInputGuard,chatSend,handRaise,handLower,panelEscKeepsMeeting};`],
  ['meet-state', [
    `goto('meeting');meetLive='meet-standup';meetPhase='live';renderMeetPhase();
     applyProto('sim','meeterror');
     var errState=meetPhase==='error'&&document.getElementById('meetPhaseCard').textContent.includes('connection failed');
     var shellRestored=getComputedStyle(document.getElementById('sidebar')).display!=='none';
     document.getElementById('meetBack').click();
     var backLobby=meetPhase==='lobby'&&document.activeElement.id==='newMeet';
     meetLive='meet-standup';meetPhase='live';renderMeetPhase();applyProto('sim','meeterror');
     document.getElementById('meetRetry').click();
     var retryConnecting=meetPhase==='connecting';
     return {errState,shellRestored,retryConnecting};`,
    `var retryLive=meetPhase==='live'&&meetError===false;
     meetLeave();
     var RV=ROOMS.find(r=>r.id==='meet-review');
     document.querySelector('.meet-trans[data-id=meet-review]').click();
     var transcript=document.getElementById('meetTransScrim').classList.contains('show')&&document.getElementById('mtList').textContent.includes('Review notes attached');
     var readOnly=!document.querySelector('#mtList input');
     document.getElementById('mtClose').click();
     var hostSimLabel=(function(){meetLive='meet-standup';meetPhase='live';renderMeetPhase();openMeetPanel('roster',document.getElementById('mkRoster'));var ok=document.getElementById('mpBody').textContent.includes('Host controls (simulated)');closeMeetPanel();meetLeave();return ok;})();
     return {retryLive,transcript,readOnly,hostSimLabel};`
  ]],
  ['meet-exit', [
    `goto('meeting');var j=document.querySelector('.meet-join');j.focus();j.click();document.getElementById('pjJoin').click();return {joined:meetPhase==='connecting'};`,
    `var live=meetPhase==='live';var lv=document.getElementById('mkLeave');lv.dispatchEvent(new PointerEvent('pointerdown',{bubbles:true,cancelable:true}));lv.dispatchEvent(new PointerEvent('pointerleave',{bubbles:true}));
     var cancelFeedback=holdState===null&&document.getElementById('toast').textContent.includes('Hold cancelled');
     var guidance=getComputedStyle(document.getElementById('meetExitHint')).display!=='none';
     lv.dispatchEvent(new PointerEvent('pointerdown',{bubbles:true,cancelable:true}));
     var barVisible=getComputedStyle(document.getElementById('holdBar')).display==='block';
     return {live,cancelFeedback,guidance,barVisible};`,
    `return {p1:!!holdState&&holdState.prog>0.3&&holdState.prog<0.7};`,
    `return {p2:!!holdState&&holdState.prog>0.7&&holdState.prog<1};`,
    `var holdCompletes=meetPhase==='lobby'&&document.body.className.indexOf('meet-live')<0;return {holdCompletes};`,
    `var ro=document.querySelector('.meet-reopen');if(ro)ro.click();var j2=document.querySelector('.meet-join');j2.focus();j2.click();document.getElementById('pjJoin').click();return {rejoined:meetPhase==='connecting'};`,
    `document.body.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true,cancelable:true}));
     var escBar=getComputedStyle(document.getElementById('holdBar')).display==='block'&&!!escHold;return {escBar};`,
    `return {e1:!!escHold&&escHold.prog>0.3&&escHold.prog<0.7};`,
    `return {e2:!!escHold&&escHold.prog>0.7&&escHold.prog<1};`,
    `var escCompletes=meetPhase==='lobby';var barHidden=getComputedStyle(document.getElementById('holdBar')).display==='none';
     document.body.classList.add('meet-live');document.body.classList.add('meet-hideov');
     var hintExempt=getComputedStyle(document.getElementById('meetExitHint')).display!=='none';
     document.body.classList.remove('meet-hideov');document.body.classList.remove('meet-live');
     return {escCompletes,barHidden,hintExempt};`
  ]],
  ['meet-polish', `goto('meeting');meetLive='meet-standup';meetPhase='live';renderMeetPhase();
    document.getElementById('mkLayout').click();
    var speakerRender=document.getElementById('meetTiles').className==='speaker'&&!!document.querySelector('#meetTiles .mtile.main')&&document.querySelectorAll('#meetRail .mtile').length>0;
    var RP=meetRoom();while(RP.parts.length<10)RP.parts.push({n:'x'+RP.parts.length,host:false,muted:true,cam:false,hand:false,conn:'ok'});
    meetPage=1;renderMeetTiles();
    var offPageIndicator=document.getElementById('meetPager').textContent.includes('speaker on another page');
    meetPage=0;renderMeetTiles();
    var pipCam=document.getElementById('meetPip').innerHTML.includes('linear-gradient');
    document.getElementById('mkCam').click();
    var pipAvatar=document.getElementById('meetPip').innerHTML.includes('avatar-big');
    document.getElementById('mkCam').click();
    var tileOverlays=!!document.querySelector('.mtile .conn.lost .ring')&&document.getElementById('meetTiles').textContent.includes('no signal')&&!!document.querySelector('.mtile .conn.weak .bar3');
    var contextInfo=document.getElementById('meetNameChip').textContent.includes('Ops standup')&&document.getElementById('meetCount').textContent.includes('in room')&&/[0-9][0-9]:[0-9][0-9]/.test(document.getElementById('meetClock').textContent);
    var mr=document.getElementById('mkRoster');mr.focus();mr.click();
    var panelTabsAria=document.getElementById('mpTabs').getAttribute('role')==='tablist'&&document.getElementById('mpTabRoster').getAttribute('aria-selected')==='true';
    document.getElementById('mpTabRoster').dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowRight',bubbles:true,cancelable:true}));
    var tabRoving=document.getElementById('mpTabChat').getAttribute('aria-selected')==='true';
    var tilesAria=document.querySelector('.mtile').getAttribute('role')==='group'&&(document.querySelector('.mtile').getAttribute('aria-label')||'').includes('connection');
    var pipShift=getComputedStyle(document.getElementById('meetPip')).right==='312px';
    closeMeetPanel();
    window.meetChatIncoming('ops1','ping');
    var chatUnread=document.getElementById('mpChatBadge').hidden===false;
    document.getElementById('mkChat').click();
    var unreadCleared=document.getElementById('mpChatBadge').hidden===true;
    closeMeetPanel();
    holdStart('leave');escHoldStart();
    var holdArbitration=holdState===null&&!!escHold;escHoldCancel();
    var RV=ROOMS.find(r=>r.id==='meet-review');RV.state='live';meetLive=RV.id;meetPhase='live';renderMeetPhase();
    RV.parts.push({n:'v2',host:false,muted:false,cam:false,hand:false,conn:'ok'});renderMeetTiles();
    var spBefore=document.querySelector('.mtile.speaking').dataset.pn;speakerTick();
    var voiceSpeaking=document.querySelector('.mtile.speaking').dataset.pn!==spBefore;
    meetLive='meet-standup';meetPhase='live';renderMeetPhase();
    var RR=meetRoom();RR.state='ended';renderMeetPhase();
    var guestEnded=document.getElementById('meetPhaseCard').textContent.includes('ended by host');
    document.getElementById('meetEndBack').click();
    var ro=document.querySelector('.meet-reopen');if(ro)ro.click();
    var reopenResets=RR.parts.length===1&&RR.chat.length===0&&(RR.transcript||[]).length>=0;
    location.hash='#meet-standup';consumeMeetHash();
    var deepLink=document.getElementById('prejoinScrim').classList.contains('show');
    document.getElementById('pjCancel').click();
    return {speakerRender,offPageIndicator,pipCam,pipAvatar,tileOverlays,contextInfo,panelTabsAria,tabRoving,tilesAria,pipShift,chatUnread,unreadCleared,holdArbitration,voiceSpeaking,guestEnded,reopenResets,deepLink};`],
  ['meet-polish-narrow', `goto('meeting');meetLive='meet-standup';meetPhase='live';renderMeetPhase();
    var mr=document.getElementById('mkRoster');mr.focus();mr.click();
    var lv=document.getElementById('mkLeave');var r=lv.getBoundingClientRect();
    var el=document.elementFromPoint(r.left+r.width/2,r.top+r.height/2);
    var leaveReachable=!!el&&(el===lv||lv.contains(el));
    var pipHidden=getComputedStyle(document.getElementById('meetPip')).display==='none';
    var noOv=document.documentElement.scrollWidth<=innerWidth;
    closeMeetPanel();meetLeave();
    return {leaveReachable,pipHidden,noOv};`, 600],
  ['remote-lobby', [
    `goto('remote');
     var cards=document.querySelectorAll('#remGrid .dev-row').length===8;
     var simTag=document.querySelector('.rem-listhead').textContent.includes('simulated');
     var ob=document.querySelector('.dev-row[data-id=DESK-03]');ob.focus();ob.click();
     var modeChip=document.getElementById('remLauncher').textContent.includes('desktop');
     var holderBadge=document.querySelector('.dev-row[data-id=DESK-03]').textContent.includes('owner: you');
     document.querySelector('#remLauncher [data-cap=control]').click();
     var loading=document.getElementById('remGrid').textContent.includes('Loading');
     return {cards,simTag,modeChip,holderBadge,loading};`,
    `var sessOpen=!document.getElementById('remSession').hidden&&document.getElementById('remLobby').hidden;
     var viewOnly=document.getElementById('remStateChip').textContent.includes('VIEW-ONLY');
     var leaseState=document.getElementById('hudLease').textContent.includes('lease idle');
     remSession='ctrl-VH-11';renderRemSession();
     var pairViewFeed=document.getElementById('remPairNote').textContent.includes('view-wall');
     var videoBg=getComputedStyle(document.getElementById('remTile')).backgroundColor!=='rgba(0, 0, 0, 0)';
     remLeaveSession();
     var backLobby=!document.getElementById('remLobby').hidden&&document.getElementById('remSession').hidden;
     applyProto('sim','remoteerror');
     var errState=document.getElementById('remDevEmpty').textContent.includes('unreachable');
     document.getElementById('remRetry').click();
     var retryLoading=document.getElementById('remGrid').textContent.includes('Loading');
     return {sessOpen,viewOnly,leaseState,pairViewFeed,videoBg,backLobby,errState,retryLoading};`,
    `var retryWorks=document.querySelectorAll('#remGrid .dev-row').length===8;
     goto('rooms');var noObject=!document.getElementById('roomBody').textContent.includes('[object Object]');
     return {retryWorks,noObject};`
  ]],
['remote-lease', `goto('remote');remSession='desk-ops';renderRemSession();
    var chipView=document.getElementById('remStateChip').textContent==='VIEW-ONLY';
    document.getElementById('remTile').click();document.getElementById('cfmOk').click();
    var chipCaptured=document.getElementById('remStateChip').textContent==='CAPTURED'&&remRoom().holder===CUR_USER&&document.getElementById('remEscHint').textContent.includes('Esc');
    var r=remRoom();r.holder='admin.x';remCaptured=false;renderRemState();
    document.getElementById('remTile').click();
    var captureGuardOtherHolder=remCaptured===false&&document.getElementById('toast').textContent.includes('admin.x');
    r.holder=null;remCapture();
    document.body.dispatchEvent(new KeyboardEvent('keydown',{key:'w',bubbles:true,cancelable:true}));
    var inputOpensLease=r.lease.state==='held'&&remKeys.has('w');
    leaseTick();var renewTick=r.lease.state==='held'&&remLeaseTtl===300;
    document.body.dispatchEvent(new KeyboardEvent('keyup',{key:'w',bubbles:true,cancelable:true}));
    leaseTick();var graceStart=r.lease.state==='grace'&&remGraceTtl===1000;
    for(var i=0;i<4;i++)leaseTick();
    var graceCountdown=r.lease.state==='grace'&&remGraceTtl===800&&document.getElementById('hudLease').textContent.includes('800')&&getComputedStyle(document.getElementById('hudLease')).display!=='none';
    for(var j=0;j<15;j++)leaseTick();
    var warnDualChannel=document.getElementById('hudLease').classList.contains('grace-warn');
    for(var k=0;k<3;k++)leaseTick();
    var ttlExpireReleasedFrame=r.lease.state==='expired'&&document.getElementById('remReleased').classList.contains('show')&&remKeys.size===0&&remCaptured===false&&document.getElementById('remStateChip').textContent==='VIEW-ONLY';
    remCapture();document.body.dispatchEvent(new KeyboardEvent('keydown',{key:'a',bubbles:true,cancelable:true}));
    applyProto('sim','remws');
    var wsDropReleasedFrame=r.conn==='lost'&&remCaptured===false&&document.getElementById('remReleased').classList.contains('show');
    applyProto('sim','remwsok');
    var reconnectViewOnly=r.conn==='ok'&&remCaptured===false&&document.getElementById('remStateChip').textContent==='VIEW-ONLY';
    remCapture();document.body.dispatchEvent(new KeyboardEvent('keydown',{key:'s',bubbles:true,cancelable:true}));
    document.body.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true,cancelable:true}));
    var escRelease=remCaptured===false&&remKeys.size===0&&document.getElementById('remStateChip').textContent==='VIEW-ONLY';
    var hudFive=['hudLat','hudJit','hudLease','hudInput','hudConn'].every(id=>document.getElementById(id).textContent.length>0&&getComputedStyle(document.getElementById(id)).display!=='none');
    document.body.classList.add('rem-hideov');
    var hExempt=getComputedStyle(document.getElementById('remStateChip')).display!=='none'&&getComputedStyle(document.getElementById('hudLease')).display!=='none'&&getComputedStyle(document.getElementById('hudConn')).display!=='none'&&getComputedStyle(document.getElementById('remFeedLabel')).display==='none';
    document.body.classList.remove('rem-hideov');
    remLeaveSession();
    return {chipView,chipCaptured,captureGuardOtherHolder,inputOpensLease,renewTick,graceStart,graceCountdown,warnDualChannel,ttlExpireReleasedFrame,wsDropReleasedFrame,reconnectViewOnly,escRelease,hudFive,hExempt};`],
  ['remote-lease-narrow', `goto('remote');remSession='desk-ops';renderRemSession();remCapture();
    var wrap=getComputedStyle(document.getElementById('remHud')).flexWrap==='wrap';
    var noOv=document.documentElement.scrollWidth<=innerWidth;
    remLeaveSession();
    return {wrap,noOv};`, 600],
  ['remote-timing', [
    `goto('remote');remSession='desk-ops';renderRemSession();remCapture();document.body.dispatchEvent(new KeyboardEvent('keydown',{key:'w',bubbles:true,cancelable:true}));return {held:remRoom().lease.state==='held'};`,
    `document.body.dispatchEvent(new KeyboardEvent('keyup',{key:'w',bubbles:true,cancelable:true}));return {renew300:remLeaseTtl===300};`,
    `return {grace:remRoom().lease.state==='grace',graceMid:remRoom().lease.state==='grace'&&remGraceTtl>300&&remGraceTtl<1000};`,
    `return {graceLate:remRoom().lease.state==='grace'&&remGraceTtl<=300};`,
    `var expired=remRoom().lease.state==='expired';return {expired1s:expired&&remCaptured===false&&document.getElementById('remReleased').classList.contains('show')};`,
    `return {bannerStill:document.getElementById('remReleased').classList.contains('show')};`,
    `return {midState:['expired','idle'].includes(remRoom().lease.state)};`,
    `return {midState2:['expired','idle'].includes(remRoom().lease.state)};`,
    `return {idleBack:remRoom().lease.state==='idle'};`
  ]],
  ['remote-estop', `goto('remote');remSession='desk-ops';renderRemSession();remCapture();var r=remRoom();
    var estopVisible=getComputedStyle(document.getElementById('remEstop')).display==='flex';
    var coarse44=[...document.querySelectorAll('style')].some(st=>st.textContent.includes('(pointer:coarse){#remEstop'));
    var es=document.getElementById('remEstop');
    es.dispatchEvent(new KeyboardEvent('keydown',{key:' ',bubbles:true,cancelable:true}));estopHoldTick();
    var kbHold=remEstopProg>0;
    es.dispatchEvent(new KeyboardEvent('keyup',{key:' ',bubbles:true,cancelable:true}));
    var kbCancel=remEstopProg===0&&document.getElementById('toast').textContent.includes('Hold cancelled');
    es.dispatchEvent(new PointerEvent('pointerdown',{bubbles:true,cancelable:true}));estopHoldTick();
    es.dispatchEvent(new PointerEvent('pointerleave',{bubbles:true}));
    var cancelFeedback=remEstopProg===0&&document.getElementById('toast').textContent.includes('Hold cancelled');
    remEstopStart();for(var i=0;i<3;i++)estopHoldTick();remEstopCancel();
    var shortHoldNoLatch=r.estop===false;
    myRole='viewer';renderRemSession();
    var capabilityGate=document.getElementById('remEstop').getAttribute('aria-disabled')==='true';
    myRole='admin';renderRemSession();
    remCapture();remKeys.add('w');remInput();
    remEstopStart();for(var j=0;j<8;j++)estopHoldTick();
    var hold400Latch=r.estop===true&&document.body.classList.contains('rem-latched');
    var pm=getComputedStyle(document.getElementById('remPerimeter'));
    var perimeter4px=pm.borderTopWidth==='4px'&&pm.pointerEvents==='none'&&pm.position==='fixed';
    var stoppedBanner=getComputedStyle(document.getElementById('remStopped')).display!=='none'&&document.getElementById('remStopped').textContent.includes('STOPPED');
    document.body.classList.add('rem-hideov');
    var neverHide=getComputedStyle(document.getElementById('remEstop')).display==='flex'&&getComputedStyle(document.getElementById('remStopped')).display==='block';
    document.body.classList.remove('rem-hideov');
    remCapture();var inputGuardLatched=remCaptured===false;
    for(var k=0;k<25;k++)leaseTick();
    var ttlExpireWhileHeld=r.lease.state==='expired'&&remKeys.size===0;
    myRole='viewer';r.holder='admin.x';renderRemSession();
    var resumeAuthViewer=document.getElementById('remResumeOpen').getAttribute('aria-disabled')==='true';
    myRole='admin';renderRemSession();
    document.getElementById('remResumeOpen').click();
    var resumeFocus=document.activeElement.id==='remResumeHold';
    document.getElementById('remResumeHold').dispatchEvent(new PointerEvent('pointerdown',{bubbles:true,cancelable:true}));
    for(var m=0;m<8;m++)resumeHoldTick();
    var resumeHeld=remResumeHeld===true&&document.getElementById('remResumeName').disabled===false;
    document.getElementById('remResumeName').value='WRONG';document.getElementById('remResumeName').dispatchEvent(new Event('input',{bubbles:true}));
    var wrongNameBlocked=document.getElementById('remResumeOk').disabled===true&&document.getElementById('remResumeErr').textContent.includes('does not match');
    document.getElementById('remResumeName').value=r.asset;document.getElementById('remResumeName').dispatchEvent(new Event('input',{bubbles:true}));
    var typedOk=document.getElementById('remResumeOk').disabled===false;
    document.getElementById('remResumeOk').click();
    var unlatched=r.estop===false&&!document.body.classList.contains('rem-latched');
    var auditBoth=AUDIT.some(a=>a.action==='estop.latch')&&AUDIT.some(a=>a.action==='estop.release');
    remLeaveSession();
    return {estopVisible,coarse44,kbHold,kbCancel,cancelFeedback,shortHoldNoLatch,capabilityGate,hold400Latch,perimeter4px,stoppedBanner,neverHide,inputGuardLatched,ttlExpireWhileHeld,resumeAuthViewer,resumeFocus,resumeHeld,wrongNameBlocked,typedOk,unlatched,auditBoth};`],
  ['remote-handover', `myRole='admin';goto('remote');remSession='desk-ops';renderRemSession();var r=remRoom();
    document.getElementById('remBall').click();document.getElementById('remSecRow').click();
    var holderBadge=document.getElementById('bmHolder').textContent==='no holder'&&getComputedStyle(document.getElementById('bmHolder')).display!=='none';
    remCapture();renderRemReq();
    var holderBadgeCtl=document.getElementById('bmHolder').textContent.includes('dispatch.x')&&getComputedStyle(document.getElementById('bmHolder')).display!=='none';
    r.holder='admin.x';remCaptured=false;renderRemState();
    document.getElementById('remTile').click();
    var requestNoSeize=r.requests.some(q=>q.from===CUR_USER)&&r.holder==='admin.x'&&remCaptured===false;
    r.holder=CUR_USER;r.requests=[{from:'admin.x',at:Date.now()}];renderRemReq();
    var notifyPersistent=document.getElementById('remReqChip').classList.contains('show')&&document.getElementById('remReqTxt').textContent.includes('admin.x')&&document.getElementById('remReqChip').getAttribute('role')==='alertdialog';
    r.requests[0].at=Date.now()-31000;leaseTick();
    var timeoutFeedback=r.requests.length===0&&document.getElementById('toast').textContent.includes('timed out');
    remCaptured=true;remKeys.add('w');r.requests=[{from:'admin.x',at:Date.now()}];
    acceptHandover('admin.x');
    var acceptFrames=r.holder==='admin.x'&&remCaptured===false&&remKeys.size===0&&document.getElementById('remReleased').classList.contains('show')&&AUDIT.some(a=>a.action==='control.handover');
    r.holder=CUR_USER;r.requests=[{from:'ops1',at:Date.now()}];renderRemReq();
    denyHandover('ops1');
    var denyFeedback=r.requests.length===0&&document.getElementById('toast').textContent.includes('denied')&&AUDIT.some(a=>a.action==='control.handover.deny');
    r.holder='admin.x';renderRemReq();
    forceRelease();
    var forceConfirm=document.getElementById('cfmScrim').classList.contains('show');
    document.getElementById('cfmOk').click();
    var forceAudit=r.holder===null&&AUDIT.some(a=>a.action==='control.force')&&document.getElementById('remReleased').classList.contains('show');
    var forceFreeForAll=r.holder===null&&remCaptured===false;
    remCapture();var freeCapture=r.holder===CUR_USER;
    r.holder=null;remCaptured=false;renderRemReq();
    var noHolderGuard=document.getElementById('bmForce').getAttribute('aria-disabled')==='true';
    forceRelease();var noHolderToast=document.getElementById('toast').textContent.includes('No active holder');
    remCapture();remKeys.add('w');remInput();remKeys.clear();for(var i=0;i<25;i++)leaseTick();
    var expiredFreeForAll=r.holder===null;
    remLeaveSession();
    return {holderBadge,holderBadgeCtl,requestNoSeize,notifyPersistent,timeoutFeedback,acceptFrames,denyFeedback,forceConfirm,forceAudit,forceFreeForAll,freeCapture,noHolderGuard,noHolderToast,expiredFreeForAll};`],
  ['remote-input', `myRole='admin';goto('remote');remSession='ctrl-VH-11';renderRemSession();remRoom().conn='ok';remCapture();var r=remRoom();
    var c0=remInputCount;document.body.dispatchEvent(new KeyboardEvent('keydown',{key:'w',bubbles:true,cancelable:true}));
    var kbdCount=remInputCount>c0;
    var c1=remInputCount;document.getElementById('remTile').dispatchEvent(new MouseEvent('mousemove',{bubbles:true}));
    var mouseCount=remInputCount>c1;
    var c2=remInputCount;window.__gamepadSim([0.8,0,0,0],[false,false,false,false]);gamepadTick();
    var gamepadAxes=remInputCount>c2;
    var sameLease=r.lease.state==='held';
    var mp=document.getElementById('remMapPanel');
    var mappingVisible=getComputedStyle(mp).display==='block'&&mp.textContent.includes('Left stick')&&mp.textContent.includes('Y (hold)');
    document.getElementById('remMapToggle').click();
    var expand1=!mp.classList.contains('collapsed')&&document.getElementById('remMapToggle').getAttribute('aria-expanded')==='true';
    document.getElementById('remMapToggle').click();
    var mappingNoEscLoop=mp.classList.contains('collapsed')&&remCaptured===true;
    window.__gamepadSim([0,0,0,0],[false,false,false,true]);gamepadTick();estopHoldTick();
    var yEstopHold=remEstopProg>0;
    for(var i=0;i<8;i++)estopHoldTick();
    var yLatched=r.estop===true;remUnlatch();
    var tg=document.getElementById('remTouch');
    var touchGatedReason=tg.getAttribute('aria-disabled')==='true'&&(tg.dataset.reason||'').includes('unproven')&&tg.getAttribute('aria-describedby')==='guardTip';
    r.holder=null;renderRemReq();
    var guardDescribedBy=document.getElementById('bmForce').getAttribute('aria-describedby')==='guardTip';
    tg.dispatchEvent(new MouseEvent('click',{bubbles:true}));
    var tapTip=document.getElementById('guardTip').textContent.includes('unproven');
    var videoBgToken=getComputedStyle(document.getElementById('remTile')).backgroundColor!=='rgba(0, 0, 0, 0)';
    document.body.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true,cancelable:true}));
    var escNotSwallowed=remCaptured===false;
    remSession='desk-ops';renderRemSession();
    var deskNoMap=getComputedStyle(document.getElementById('remMapPanel')).display==='none';
    remLeaveSession();
    return {kbdCount,mouseCount,gamepadAxes,sameLease,mappingVisible,expand1,mappingNoEscLoop,yEstopHold,yLatched,touchGatedReason,guardDescribedBy,tapTip,videoBgToken,escNotSwallowed,deskNoMap};`],
  ['remote-fixes', `myRole='admin';goto('remote');remSession='desk-ops';renderRemSession();var r=remRoom();
    remCapture();remKeys.add('w');remInput();remKeys.clear();leaseTick();
    window.__gamepadSim([0.8,0,0,0],[false,false,false,false]);leaseTick();
    var gamepadGraceHeld=r.lease.state==='held';
    window.__gamepadSim([0,0,0,0],[false,false,false,false]);
    applyProto('sim','remws');remCaptured=false;remCapture();
    var connLostCaptureBlocked=remCaptured===false;
    remEstopStart();for(var i=0;i<8;i++)estopHoldTick();
    var connLostEstopBlocked=r.estop===false;
    var healthBannerNotStopped=getComputedStyle(document.getElementById('remHealth')).display==='block'&&getComputedStyle(document.getElementById('remStopped')).display==='none';
    applyProto('sim','remwsok');remCapture();remEstopStart();for(var j=0;j<8;j++)estopHoldTick();
    document.getElementById('remResumeOpen').click();
    document.getElementById('remResumeHold').dispatchEvent(new PointerEvent('pointerdown',{bubbles:true,cancelable:true}));resumeHoldTick();resumeHoldTick();
    document.getElementById('remResumeHold').dispatchEvent(new PointerEvent('pointerleave',{bubbles:true}));
    var resumeDragCancel=remResumeProg===0&&remResumeIv===null;
    document.getElementById('remResumeHold').dispatchEvent(new PointerEvent('pointerdown',{bubbles:true,cancelable:true}));resumeHoldTick();
    document.body.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true,cancelable:true}));
    var resumeEscClears=remResumeIv===null&&!document.getElementById('remResumeScrim').classList.contains('show');
    remUnlatch();remCapture();remEstopStart();
    ROOMS.push({id:'desk-2',kind:'desk',name:'Desk two',parts:1,state:'idle',asset:'DESK-09',mode:'desk',holder:null,lease:{state:'idle',ttl:0},estop:false,conn:'ok',viewers:[],requests:[],pairView:null});
    remSession='desk-2';for(var k=0;k<8;k++)estopHoldTick();
    var crossRoomLatchGuard=ROOMS.find(x=>x.id==='desk-2').estop===false&&r.estop===false;
    remSession='desk-ops';renderRemSession();remCapture();remEstopStart();for(var m=0;m<8;m++)estopHoldTick();
    var latchKeepsHolder=r.holder===CUR_USER&&r.estop===true;
    var holderResumeReachable=document.getElementById('remResumeOpen').getAttribute('aria-disabled')!=='true';
    forceRelease();document.getElementById('cfmOk').click();
    var forceLatchedInert=r.holder===null&&r.lease.state==='inert';
    r.holder=CUR_USER;remLeaveSession();
    var backLatchedClearsHolder=r.holder===null;
    goto('remote');remSession='desk-ops';renderRemSession();remCapture();goto('rooms');
    var navLeaseReleased=remCaptured===false&&AUDIT.some(a=>a.action==='control.released');
    var navBodyClassesCleared=!document.body.classList.contains('rem-session')&&!document.body.classList.contains('rem-latched');
    goto('remote');
    var lobbyStoppedChip=document.getElementById('remGrid').textContent.includes('STOPPED');
    remSession='desk-ops';renderRemSession();remUnlatch();remCapture();openDevDrawer(DEVICES[0].id);
    document.body.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true,cancelable:true}));
    var escChainAfterDrawer=!document.getElementById('devDrawer').classList.contains('open')&&remCaptured===true;
    meetPhase='live';document.body.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true,cancelable:true}));
    var escMeetBranchPageOwned=escHold===null&&remCaptured===false;meetPhase='lobby';
    remCapture();document.body.dispatchEvent(new KeyboardEvent('keydown',{key:'Tab',bubbles:true,cancelable:true}));
    var phantomKeysExcluded=!remKeys.has('Tab');
    document.body.dispatchEvent(new KeyboardEvent('keydown',{key:'c',ctrlKey:true,bubbles:true,cancelable:true}));
    var modifierCombosExcluded=!remKeys.has('c');
    remRelease('test');remUnlatch();var tl=document.getElementById('remTile');tl.focus();
    tl.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true,cancelable:true}));
    var tileKeyboardCapture=document.getElementById('cfmScrim').classList.contains('show');
    var seizeConfirmFocus=document.activeElement.id==='cfmCancel';
    document.getElementById('cfmOk').click();
    var seizeOkCaptured=remCaptured===true;
    remRelease('test');r.conn='lost';tl.click();
    var seizeOkRevalidate=!document.getElementById('cfmScrim').classList.contains('show');
    r.conn='ok';
    setTheme('light');
    function lum(hex){var c=hex.replace('#','');var v=[0,2,4].map(function(o){var x=parseInt(c.substr(o,2),16)/255;return x<=0.03928?x/12.92:Math.pow((x+0.055)/1.055,2.4);});return 0.2126*v[0]+0.7152*v[1]+0.0722*v[2];}
    var cs=getComputedStyle(document.documentElement);var l1=lum(cs.getPropertyValue('--video-ink').trim());var l2=lum(cs.getPropertyValue('--video-bg').trim());
    var lightContrastFormula=(Math.max(l1,l2)+0.05)/(Math.min(l1,l2)+0.05)>=4.5;
    remCapture();remKeys.add('w');remInput();remKeys.clear();leaseTick();
    var graceEnter=document.getElementById('remLive').textContent.includes('release in 1 second');
    for(var q=0;q<15;q++)leaseTick();
    var graceLiveTransitions=graceEnter&&document.getElementById('remLive').textContent.includes('Warning');
    var touchReleaseHintDual=document.getElementById('remEscHint').textContent.includes('Esc')&&getComputedStyle(document.getElementById('remEscHint')).display!=='none'&&(document.getElementById('remEscHint').title.includes('Esc or Release')||document.getElementById('remDetailsBody').textContent.includes('Esc or Release'));
    r.holder=CUR_USER;r.requests=[{from:'admin.x',at:Date.now()}];renderRemReq();
    var reqFocus=document.activeElement.id==='remReqAccept';
    denyHandover('admin.x');
    var reqFocusRestore=reqFocus&&['remTake','remRelease','remTile'].includes(document.activeElement.id);
    var statusPopFixed=!!document.getElementById('statusPop')&&document.getElementById('statusPop').classList.contains('tb-pop');
    remLeaveSession();
    return {gamepadGraceHeld,connLostCaptureBlocked,connLostEstopBlocked,healthBannerNotStopped,resumeDragCancel,resumeEscClears,crossRoomLatchGuard,latchKeepsHolder,holderResumeReachable,forceLatchedInert,backLatchedClearsHolder,navLeaseReleased,navBodyClassesCleared,lobbyStoppedChip,escChainAfterDrawer,escMeetBranchPageOwned,phantomKeysExcluded,modifierCombosExcluded,tileKeyboardCapture,seizeConfirmFocus,seizeOkCaptured,seizeOkRevalidate,lightContrastFormula,graceLiveTransitions,touchReleaseHintDual,reqFocusRestore,statusPopFixed};`],
  ['remote-layout', `myRole='admin';goto('remote');remSession='desk-ops';renderRemSession();var r=remRoom();
    var shellHidden=getComputedStyle(document.getElementById('sidebar')).display==='none'&&getComputedStyle(document.getElementById('topbar')).display==='none';
    var stg=document.getElementById('remStage').getBoundingClientRect();var tl=document.getElementById('remTile').getBoundingClientRect();
    var tileFillViewport=stg.height>400&&tl.width>600;
    document.getElementById('remBall').click();
    var top=document.getElementById('remBallPop');
    var topbarGroups=['remFs3','remDisplayRow','remInputRow','remSecRow'].every(function(id){return !!top.querySelector('#'+id);});
    document.getElementById('remDisplayRow').click();
    var overflowTbPop=document.getElementById('remDisplayPop').classList.contains('tb-pop')&&!document.getElementById('remDisplayPop').hidden;
    document.body.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true,cancelable:true}));
    var overflowEsc=document.getElementById('remDisplayPop').hidden&&!document.getElementById('remBallPop').hidden;
    var rb=document.querySelector('#remSafeBar .inner');var mb=document.getElementById('meetBar');
    var bottomBarSharedShell=rb.classList.contains('session-bar')&&mb.classList.contains('session-bar');
    var stateSlotLiveOff=!document.getElementById('remStateChip').getAttribute('aria-live');
    remCapture();remKeys.add('w');remInput();remKeys.clear();leaseTick();
    var graceMirror=!document.getElementById('hudLease').hidden&&document.getElementById('hudLease').textContent.includes('release in')&&getComputedStyle(document.getElementById('hudLease')).display!=='none';
    document.getElementById('remFs3').click();
    var fullscreenRealOrStub=document.fullscreenElement===document.getElementById('remSession')||window.__fsStub===true||window.__fsPending===true;
    var er=document.getElementById('remEstop').getBoundingClientRect();
    var estopVisibleInFullscreen=getComputedStyle(document.getElementById('remEstop')).display==='flex'&&er.width>0;
    document.getElementById('remDisplayRow').click();document.getElementById('remFitRow').click();
    document.getElementById('remZoomOrig').click();
    var zoomOriginal=document.getElementById('remStage').classList.contains('original')&&document.getElementById('remZoomOrig').getAttribute('aria-pressed')==='true';
    document.getElementById('remZoomFit').click();
    var zoomFit=!document.getElementById('remStage').classList.contains('original')&&document.getElementById('remZoomFit').getAttribute('aria-pressed')==='true';
    var sessionTimer=/^[0-9][0-9]:[0-9][0-9]/.test(document.getElementById('remClock').textContent)&&getComputedStyle(document.getElementById('remClock')).display!=='none';
    document.getElementById('remDisplayRow').click();document.getElementById('remAdvRow').click();document.getElementById('remOsdBtn3').click();
    var osdTogglePressed=document.body.classList.contains('rem-osd')&&document.getElementById('remOsdBtn3').getAttribute('aria-pressed')==='true'&&getComputedStyle(document.getElementById('remOsd')).display==='flex';
    var c0=remInputCount;document.getElementById('remInputRow').click();document.getElementById('remKeysRow2').click();
    document.querySelector('#remKeysPop [data-sk="Ctrl+Alt+Del"]').click();
    var specialKeysLease=remInputCount>c0&&AUDIT.some(function(a){return a.action==='control.specialkey';});
    var ec=document.getElementById('remEstop').getBoundingClientRect();
    var el=document.elementFromPoint(ec.left+ec.width/2,ec.top+ec.height/2);
    var estopNeverCovered=!!el&&(el.id==='remEstop'||document.getElementById('remEstop').contains(el));
    document.body.classList.add('rem-hideov');
    var hExemptions=getComputedStyle(document.getElementById('remStateChip')).display!=='none'&&getComputedStyle(document.getElementById('hudLease')).display!=='none'&&getComputedStyle(document.getElementById('hudConn')).display!=='none'&&getComputedStyle(document.getElementById('remEstop')).display==='flex'&&getComputedStyle(document.getElementById('remFeedLabel')).display==='none';
    r.conn='lost';renderRemSession();
    var healthBannerNeverHide=getComputedStyle(document.getElementById('remHealth')).display==='block';
    r.conn='ok';document.body.classList.remove('rem-hideov');renderRemSession();
    remLeaveSession();
    return {shellHidden,tileFillViewport,topbarGroups,overflowTbPop,overflowEsc,bottomBarSharedShell,stateSlotLiveOff,graceMirror,fullscreenRealOrStub,estopVisibleInFullscreen,zoomOriginal,zoomFit,sessionTimer,osdTogglePressed,specialKeysLease,estopNeverCovered,hExemptions,healthBannerNeverHide};`],
  ['remote-layout-narrow', `myRole='admin';goto('remote');remSession='desk-ops';renderRemSession();
    var wideHidden=getComputedStyle(document.getElementById('remFs3')).display!=='none';
    document.getElementById('remBall').click();document.getElementById('remDisplayRow').click();
    var moreWorks=!document.getElementById('remDisplayPop').hidden;
    var noOv=document.documentElement.scrollWidth<=innerWidth;
    var barWrap=getComputedStyle(document.querySelector('#remSafeBar .inner')).flexWrap==='wrap';
    remLeaveSession();
    return {wideHidden,moreWorks,noOv,barWrap};`, 600],
  ['settings-login', `goto('settings');
    document.getElementById('liBtn').click();
    var formFocus=document.activeElement.id==='liUser'&&document.getElementById('liErr').textContent.length>0;
    var mask=document.getElementById('liPass').type==='password';
    document.getElementById('liUser').value='ops9';document.getElementById('liPass').value='x';document.getElementById('liRole').value='operator';
    document.getElementById('liBtn').click();
    var loginSwitchesRole=CUR_USER==='ops9'&&myRole==='operator'&&AUDIT.some(function(a){return a.action==='auth.login';});
    var signedInCard=document.getElementById('liStatus').textContent.includes('Signed in as ops9');
    var ttlChip=document.getElementById('page-settings').textContent.includes('TTL 24 h');
    document.getElementById('liOut').click();
    var signoutRestores=myRole==='viewer'&&document.getElementById('liStatus').textContent.includes('Signed out');
    applyProto('sim','loginerror');
    var errorLeg=document.getElementById('liErr').textContent.includes('Invalid credentials');
    var simTag=document.getElementById('page-settings').textContent.includes('simulated');
    var aboutVersion=document.getElementById('aboutVer').textContent===PROTO_VER;
    goto('signaling');document.getElementById('sigTest').click();
    var sigTestSimTag=!document.getElementById('sigResult').hidden&&document.getElementById('sigResult').textContent.includes('simulated');
    return {formFocus,mask,loginSwitchesRole,signedInCard,ttlChip,signoutRestores,errorLeg,simTag,aboutVersion,sigTestSimTag};`],
  ['settings-index', `goto('settings');
    var sixRows=document.querySelectorAll('#cfgIndex .set-row').length===6;
    var valuesSim=document.getElementById('cfgIndex').textContent.includes('simulated');
    document.querySelector('#cfgIndex .cfg-open[data-go=rooms]').click();
    var deepLinkRooms=current==='rooms';
    goto('settings');
    var edits=[...document.querySelectorAll('#cfgIndex .cfg-edit')];
    var gatedReason=edits.every(function(b){return b.getAttribute('aria-disabled')==='true'&&(b.dataset.reason||'').includes('manifest');});
    var noEditAffordance=edits.every(function(b){return b.getAttribute('aria-disabled')==='true';});
    document.querySelector('#cfgIndex .cfg-open[data-go=keys]').click();
    var deepLinkKeys=current==='keys';
    return {sixRows,valuesSim,deepLinkRooms,gatedReason,noEditAffordance,deepLinkKeys};`],
  ['remote-safetyfix', `myRole='admin';goto('remote');remSession='desk-ops';renderRemSession();remCapture();
    var rl=document.getElementById('remRelease');rl.focus();
    var c0=remInputCount;rl.dispatchEvent(new KeyboardEvent('keydown',{key:' ',bubbles:true,cancelable:true}));
    var spaceOnChromeNotForwarded=!remKeys.has(' ')&&remInputCount===c0;
    var tl=document.getElementById('remTile');tl.focus();
    var c1=remInputCount;tl.dispatchEvent(new KeyboardEvent('keydown',{key:' ',bubbles:true,cancelable:true}));
    var spaceOnTileForwarded=remInputCount>c1&&remKeys.has(' ');
    remKeys.delete(' ');
    var sel=document.createElement('select');sel.id='testSel';document.querySelector('#remSafeBar .inner').appendChild(sel);sel.focus();
    sel.dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowDown',bubbles:true,cancelable:true}));
    var arrowOnSelectExcluded=!remKeys.has('ArrowDown');sel.remove();
    var er=document.getElementById('remEstop').getBoundingClientRect();
    var el=document.elementFromPoint(er.left+er.width/2,er.top+er.height/2);
    var narrow400EstopUncovered=!!el&&(el.id==='remEstop'||document.getElementById('remEstop').contains(el));
    var rr=document.getElementById('remRelease').getBoundingClientRect();
    var el2=document.elementFromPoint(rr.left+rr.width/2,rr.top+rr.height/2);
    var narrow400BarReachable=!!el2&&(el2.id==='remRelease'||document.getElementById('remRelease').contains(el2));
    remLeaveSession();
    return {spaceOnChromeNotForwarded,spaceOnTileForwarded,arrowOnSelectExcluded,narrow400EstopUncovered,narrow400BarReachable};`, 400],
  ['remote-window', `myRole='admin';goto('remote');remSession='desk-ops';renderRemSession();
    document.body.classList.add('rem-hideov');
    var safetyBarNeverHide=getComputedStyle(document.getElementById('remSafeBar')).display!=='none'&&getComputedStyle(document.getElementById('remStateChip')).display!=='none'&&getComputedStyle(document.getElementById('hudLease')).display!=='none'&&getComputedStyle(document.getElementById('remTake')).display!=='none';
    document.body.classList.remove('rem-hideov');
    var topHiddenDefault=!document.getElementById('remTop')&&!document.getElementById('remWake');
    var wakeWorks=topHiddenDefault;
    document.getElementById('remTake').click();document.getElementById('cfmOk').click();
    var capturedCarrierBall=getComputedStyle(document.getElementById('remBall')).display==='flex';
    var stateSwitchBadge=document.getElementById('remLive').textContent.length>0;
    var b=document.getElementById('remBall');b.style.left=(innerWidth-60)+'px';b.style.top=(innerHeight-60)+'px';ballDockTick();
    var forbiddenZones=parseFloat(b.style.left)<innerWidth-100;
    b.style.left='12px';b.style.top='52px';ballDockTick();var dockEdge=b.classList.contains('docked');
    b.style.left=Math.round(innerWidth/2)+'px';b.style.top=Math.round(innerHeight/2)+'px';ballDockTick();
    var ballDragDock=dockEdge&&!b.classList.contains('docked');
    b.classList.add('docked');var dim=getComputedStyle(b).opacity; b.focus();var foc=getComputedStyle(b).opacity;
    var ballDimFocusRestore=parseFloat(dim)<1&&parseFloat(foc)===1;
    document.getElementById('remBall').click();
    var menuIaTemplate=!!document.getElementById('remBallPop').querySelector('#remFs3')&&!!document.getElementById('remOsdBtn3')&&!!document.getElementById('remDisplayRow')&&!!document.getElementById('remBack3');
    document.getElementById('remDisplayRow').click();
    var gatedDisplay=document.getElementById('gQuality').getAttribute('aria-disabled')==='true';
    document.getElementById('remZoomOrig').click();
    var st=document.getElementById('remStage');
    var c0=remInputCount;
    document.getElementById('remTile').dispatchEvent(new MouseEvent('mousedown',{button:1,bubbles:true,cancelable:true}));
    document.dispatchEvent(new MouseEvent('mousemove',{movementX:-40,movementY:0,bubbles:true}));
    document.dispatchEvent(new MouseEvent('mouseup',{bubbles:true}));
    var originalScrollPanExempt=st.classList.contains('original')&&st.scrollLeft>0&&remInputCount===c0;
    document.getElementById('remZoomFit').click();
    var videoContainDefault=!st.classList.contains('original');
    document.getElementById('remInputRow').click();document.getElementById('remSessRow2').click();
    var rows=document.querySelectorAll('#remSessList [data-ss]');
    var target=[...rows].find(x=>x.dataset.ss!=='desk-ops');
    target.click();
    var sessionListSwitchReleasesOld=remSession===target.dataset.ss;
    remSession='desk-ops';renderRemSession();
    document.getElementById('remExpand').click();
    var expandDetails=!document.getElementById('remDetailsPop').hidden&&document.getElementById('remDetailsBody').textContent.includes('H.264');
    closeTbPop(null,true);
    var badgeNotStretched=!document.querySelector('#remTile .tile-live');
    var no230Constant=![...document.querySelectorAll('style')].some(sty=>/#remTile\{[^}]*230px/.test(sty.textContent));
    var connOverlay=document.getElementById('remConnecting');
    var connectingState=!!connOverlay;
    remLeaveSession();
    return {safetyBarNeverHide,topHiddenDefault,wakeWorks,capturedCarrierBall,stateSwitchBadge,forbiddenZones,ballDragDock,ballDimFocusRestore,menuIaTemplate,gatedDisplay,originalScrollPanExempt,videoContainDefault,sessionListSwitchReleasesOld,expandDetails,badgeNotStretched,no230Constant,connectingState};`],
  ['remote-launcher', `goto('remote');
    var launcherEmpty=!document.getElementById('remLauncherEmpty').hidden&&document.getElementById('remLauncher').hidden;
    var card=document.querySelector('.dev-row[data-id=DESK-03]');card.click();
    var launcherSelect=!document.getElementById('remLauncher').hidden&&document.getElementById('remLauncherTitle').textContent.length>0&&document.querySelector('.dev-row[data-id=DESK-03]').getAttribute('aria-selected')==='true';
    document.getElementById('lmCopy').click();
    var metaCopyFallback=document.getElementById('lmCode').textContent.length>0;
    var gated=document.querySelector('#remLauncher [data-cap=file]');
    gated.click();
    var tilesGatedNeverLaunch=document.getElementById('remSession').hidden&&document.getElementById('guardTip').textContent.includes('Not available yet');
    gated.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true,cancelable:true}));
    gated.click();
    var gatedEnterLeg=document.getElementById('remSession').hidden;
    gated.focus();
    var gatedReasonKeyboard=document.getElementById('guardTip').textContent.includes('Not available yet')&&gated.getAttribute('aria-describedby')==='guardTip';
    var pow=document.querySelector('#remLauncher [data-pow=reboot]');pow.click();pow.dispatchEvent(new KeyboardEvent('keydown',{key:' ',bubbles:true,cancelable:true}));pow.click();
    var powerGatedNeverFire=document.getElementById('remSession').hidden&&!AUDIT.some(function(a){return a.action.indexOf('power')===0;});
    var staleRoom=ROOMS.find(function(x){return x.id==='desk-ops';});staleRoom.conn='lost';renderLauncher();
    var launcherStaleError=document.getElementById('remLauncher').textContent.includes('offline or revoked');
    staleRoom.conn='ok';renderLauncher();
    remLoading=true;renderRemote();
    var launcherLoading=document.getElementById('remGrid').textContent.includes('Loading');
    remLoading=false;renderRemote();
    document.querySelector('#remLauncher [data-cap=control]').click();
    var tilesRealRoute=remSession==='desk-ops';
    remLoading=false;renderRemote();
    remLeaveSession();
    document.querySelector('.dev-row[data-id=DESK-03]').click();
    document.querySelector('#remLauncher [data-cap=view]').click();
    var vr=remRoom();
    var viewSessionNoTake=!document.getElementById('remSession').hidden&&document.getElementById('remLobby').hidden&&vr.kind==='view'&&document.getElementById('remTake').hidden&&document.getElementById('remRelease').hidden;
    var tile=document.getElementById('remTile');
    tile.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true,cancelable:true}));
    tile.click();
    var tileEnterNoCapture=remCaptured===false&&tile.getAttribute('aria-label').includes('View-only');
    var viewBarVariantChip=!document.getElementById('remViewHint').hidden&&document.getElementById('hudLease').hidden;
    var estopHiddenInView=getComputedStyle(document.getElementById('remEstop')).display==='none';
    document.getElementById('remViewBack').click();
    document.querySelector('.dev-row[data-id=DESK-03]').click();
    document.querySelector('#remLauncher [data-cap=camera]').click();
    var cameraViewRoute=remRoom().kind==='view'&&remRoom().mode==='camera'&&!!remRoom().feed;
    remLeaveSession();
    return {launcherEmpty,launcherSelect,metaCopyFallback,tilesGatedNeverLaunch,gatedEnterLeg,gatedReasonKeyboard,powerGatedNeverFire,launcherStaleError,launcherLoading,tilesRealRoute,viewSessionNoTake,tileEnterNoCapture,viewBarVariantChip,estopHiddenInView,cameraViewRoute};`],
  ['remote-launcher-narrow', `goto('remote');document.querySelector('.dev-row[data-id=DESK-03]').click();
    var gr=document.getElementById('remGrid').getBoundingClientRect();var lr=document.getElementById('remLauncher').getBoundingClientRect();
    var narrowFold=lr.top>gr.top&&lr.width>300;
    var noOv=document.documentElement.scrollWidth<=innerWidth;
    return {narrowFold,noOv};`, 400],
  ['remote-devlist', `goto('remote');
    var treeCounts=document.getElementById('remTree').textContent.includes('All devices')&&document.getElementById('remTree').textContent.includes('8')&&document.getElementById('remTree').textContent.includes('Yard vehicles');
    var treeNoDeadControl=!document.querySelector('#remTree [data-tw]')&&![...document.querySelectorAll('#remTree [role=option]')].some(function(o){return o.hasAttribute('aria-expanded');});
    document.querySelector('#remTree [data-g="Yard vehicles"]').click();
    var groupFilter=[...document.querySelectorAll('#remGrid .dev-row')].every(function(r){return ['VH-07','VH-11','VH-02'].includes(r.dataset.id);});
    document.querySelector('#remTree [data-g=""]').click();
    var rr=ROOMS.find(function(x){return x.id==='desk-ops';});rr.holder='admin.x';rr.estop=true;renderRemote();
    var rowTxt=document.querySelector('.dev-row[data-id=DESK-03]').textContent;
    var rowBadges=rowTxt.includes('In control')&&rowTxt.includes('This device')&&rowTxt.includes('STOPPED');
    rr.holder=null;rr.estop=false;renderRemote();
    var offRow=document.querySelector('.dev-row[data-id=VH-02]');
    var offlineDim=offRow.classList.contains('off')&&getComputedStyle(offRow).color!==getComputedStyle(document.querySelector('.dev-row[data-id=DESK-03]')).color;
    document.querySelector('.dev-row[data-id=DESK-03]').click();
    var tier2Subline=getComputedStyle(document.querySelector('.dev-row[aria-selected=false] .dev-meta')).display==='flex';
    document.getElementById('lmRename').click();
    var renameModalFocus=document.activeElement.id==='rnAlias';
    document.getElementById('rnAlias').value='Test alias';
    document.getElementById('rnTagIn').value='qa';document.getElementById('rnTagAdd').click();
    var tagsChips=document.querySelectorAll('#rnTags [data-rt]').length>=1;
    document.getElementById('rnSave').click();
    var renameApplies=document.querySelector('.dev-row[data-id=DESK-03]').textContent.includes('Test alias (');
    var aliasFormat=document.querySelector('.dev-row[data-id=DESK-03]').textContent.includes('(Ops workstation)');
    var renameAudit=AUDIT.some(function(a){return a.action==='device.rename';});
    var se=document.getElementById('remSearch');se.value='cam';se.oninput();
    var searchHit=document.querySelectorAll('#remGrid .dev-row').length===3&&document.getElementById('remHitCount').textContent.includes('3/8');
    se.value='';se.oninput();
    document.getElementById('remSortBtn').click();document.querySelector('#remSortPop [data-sort=online]').click();
    var sortOrder=document.querySelector('#remGrid .dev-row').dataset.id!=='VH-02';
    document.getElementById('remFilterBtn').click();
    document.querySelector('#remFilters [data-f=online]').click();document.querySelector('#remFilters [data-f="os:linux"]').click();
    var filterStack=[...document.querySelectorAll('#remGrid .dev-row')].every(function(r){var d=DEVICES.find(function(x){return x.id===r.dataset.id;});return d.status==='online'&&d.os==='linux';});
    document.querySelector('#remFilters [data-f=online]').click();document.querySelector('#remFilters [data-f="os:linux"]').click();
    remHistory=['CAM-01'];renderDevRows();
    document.querySelector('#remRecentRows .dev-row').click();
    var recentDirect=remSel==='CAM-01';
    rr.holder=CUR_USER;remGroup='Cameras';renderRemote();
    var activeTop=document.querySelector('#remActiveRows .dev-row').dataset.id==='DESK-03';
    rr.holder=null;remGroup=null;renderRemote();
    se.value='zzz';se.oninput();
    var emptyShown=!document.getElementById('remDevEmpty').hidden&&document.getElementById('remDevEmpty').textContent.includes('No matching');
    document.getElementById('remClearFilters').click();
    var emptyClear=document.querySelectorAll('#remGrid .dev-row').length===8&&document.activeElement.id==='remSearch';
    var simTag=document.querySelector('.rem-listhead').textContent.includes('simulated');
    document.getElementById('remGroupAdd').click();document.getElementById('rgName').value='QA group';document.getElementById('rgSave').click();
    var groupCrud=document.getElementById('remTree').textContent.includes('QA group');
    var ti=document.querySelector('#remTree [data-g=""]');ti.focus();ti.dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowDown',bubbles:true,cancelable:true}));
    var treeRoving=document.activeElement.getAttribute('role')==='option'&&document.activeElement.dataset.g!=='';
    return {treeCounts,treeNoDeadControl,groupFilter,rowBadges,offlineDim,tier2Subline,renameModalFocus,renameApplies,aliasFormat,tagsChips,renameAudit,searchHit,sortOrder,filterStack,recentDirect,activeTop,emptyShown,emptyClear,simTag,groupCrud,treeRoving};`],
  ['remote-devlist-narrow', `goto('remote');
    var row=document.querySelector('#remGrid .dev-row');
    var tier3TwoLine=getComputedStyle(row.querySelector('.dev-meta')).display==='flex'&&row.getBoundingClientRect().height>44;
    var treeChips=getComputedStyle(document.getElementById('remTree')).display==='flex';
    var noOv=document.documentElement.scrollWidth<=innerWidth;
    return {tier3TwoLine,treeChips,noOv};`, 400],
  ['remote-devlist-1600', `goto('remote');
    var meta=document.querySelector('#remGrid .dev-row .dev-meta');
    var tier1RightMeta=getComputedStyle(document.querySelector('.dev-row[aria-selected=false] .dev-meta')).display==='flex';
    var colWide=document.querySelector('.rem-lobby-left').getBoundingClientRect().width>=400;
    return {tier1RightMeta,colWide};`, 1600],
  ['tower', `goto('tower');
    var alertsPanel=document.querySelector('#towerGrid section[aria-label="Alerts"]');
    var h0=alertsPanel.getBoundingClientRect().height;
    document.querySelector('#towerGrid [data-col=talk]').click();
    var collapseFreesSpace=document.querySelector('#towerGrid section[aria-label="Alerts"]').getBoundingClientRect().height>h0;
    document.querySelector('#towerGrid [data-col=talk]').click();
    var foc=[...document.querySelectorAll('#towerGrid [data-col]')];
    var tabOrder=foc[0].getAttribute('data-col')==='wall'&&foc[1].getAttribute('data-col')==='talk'&&foc[2].getAttribute('data-col')==='alerts';
    towerPanelError('wall','simulated wall failure');
    var panelStatesIndependent=document.querySelector('#wallBody').textContent.includes('Retry')&&document.querySelectorAll('#towerTalk [data-chan]').length>0;
    document.querySelector('#wallBody [data-retry=wall]').click();
    var simTag=document.querySelector('#towerGrid').textContent.includes('simulated');
    var heldCountMatches=document.getElementById('towerHeld').textContent===GROUPS.filter(function(g){return g.state==='held';}).length+' held';
    var overflowMore=GROUPS.length>5?!!document.getElementById('twMore'):true;
    var idle=GROUPS.find(function(g){return g.state==='idle';});
    document.querySelector('#towerTalk [data-chan="'+idle.id+'"]').click();
    var ptt=document.getElementById('twPtt');
    ptt.dispatchEvent(new PointerEvent('pointerdown',{bubbles:true,cancelable:true,pointerId:7}));
    var pttSelectedChannel=idle.holder===CUR_USER&&pttHeld===true;
    var holdCountdown=document.getElementById('twCountdown').textContent.includes('s left');
    ptt.dispatchEvent(new PointerEvent('pointerup',{bubbles:true,cancelable:true,pointerId:7}));
    goto('intercom');
    var pttSameGrant=chanOpen===idle.id;
    goto('tower');
    var al0=ALERTS.filter(function(a){return !a.acked;}).length;
    var navBadgeDynamic=document.querySelector('[data-badge=tower]').textContent===String(al0);
    document.querySelector('#towerAlerts [data-ack]').click();
    var ackAudit=AUDIT.some(function(a){return a.action==='alert.ack';});
    var ackFilter=document.querySelectorAll('#towerAlerts [data-al]').length===al0-1;
    var ackFilterOff=document.getElementById('twAckFilter');ackFilterOff.click();
    var ackedMark=document.querySelector('#towerAlerts .alert-item.acked .ack-mark')!==null;
    document.getElementById('twAckFilter').click();
    var devBtn=document.querySelector('#towerAlerts [data-aldev]');
    devBtn.click();
    var alertDeepLink=current==='devices';
    document.getElementById('backTower').click();
    var backToTower=current==='tower'&&!document.getElementById('backTower').classList.contains('show');
    document.querySelector('#towerWall [data-stream]').click();
    var wallDeepLink=current==='monitor'&&document.getElementById('backTower').classList.contains('show');
    document.getElementById('backTower').click();
    return {collapseFreesSpace,tabOrder,panelStatesIndependent,simTag,heldCountMatches,overflowMore,pttSelectedChannel,holdCountdown,pttSameGrant,navBadgeDynamic,ackAudit,ackFilter,ackedMark,alertDeepLink,backToTower,wallDeepLink};`],
  ['tower-narrow', `goto('tower');
    var secs=[...document.querySelectorAll('#towerGrid section')];
    var narrowStack=secs[0].getBoundingClientRect().top<secs[1].getBoundingClientRect().top&&secs[1].getBoundingClientRect().top<secs[2].getBoundingClientRect().top;
    var ack=document.querySelector('#towerAlerts [data-ack]');
    ack.scrollIntoView({block:'center'});
    var r=ack.getBoundingClientRect();
    var el=document.elementFromPoint(r.left+r.width/2,r.top+r.height/2);
    var ackReachable=!!el&&(el===ack||ack.contains(el));
    var noOv=document.documentElement.scrollWidth<=innerWidth;
    return {narrowStack,ackReachable,noOv};`, 400],
  ['intercom-space', `goto('intercom');enterChannel((GROUPS.find(function(g){return g.state==='idle';})||GROUPS[0]).id);
    document.body.focus();
    document.body.dispatchEvent(new KeyboardEvent('keydown',{code:'Space',key:' ',bubbles:true,cancelable:true}));
    var bodySpaceNoPtt=pttHeld===false;
    var ptt=document.getElementById('chPtt');ptt.focus();
    ptt.dispatchEvent(new KeyboardEvent('keydown',{code:'Space',key:' ',bubbles:true,cancelable:true}));
    var focusSpacePtt=pttHeld===true;
    ptt.dispatchEvent(new KeyboardEvent('keyup',{code:'Space',key:' ',bubbles:true,cancelable:true}));
    var focusSpaceRelease=pttHeld===false;
    return {bodySpaceNoPtt,focusSpacePtt,focusSpaceRelease};`],
  ['remote-unify', `goto('remote');document.querySelector('.dev-row[data-id=DESK-03]').click();
    var L=document.getElementById('remLauncher');
    var nm=L.querySelector('.lm-name').getBoundingClientRect(),sb2=L.querySelector('.lm-sub').getBoundingClientRect(),cl=L.querySelector('.lm-codeline').getBoundingClientRect(),cp=document.getElementById('lmCopy').getBoundingClientRect();
    var metaThreeRows=nm.top<sb2.top&&sb2.top<cl.top&&Math.abs(cp.top-cl.top)<6;
    var tile=document.querySelector('#remLauncher .rem-tiles:not(.rem-power) .rem-tile');
    var tileLabelNowrap=tile.getBoundingClientRect().height<80;
    var rowIconTile=getComputedStyle(document.querySelector('#remGrid .dev-ic')).backgroundColor!==('rgba(0, 0, 0, 0)');
    var recentHeadPlain=getComputedStyle(document.getElementById('remRecentToggle')).borderTopWidth==='0px';
    var devGo=!!document.querySelector('#remGrid .dev-row .dev-go');
    return {metaThreeRows,tileLabelNowrap,rowIconTile,recentHeadPlain,devGo};`],
  ['remote-visual', `goto('remote');
    function lum(c){var m=(c||'').match(/[0-9.]+/g);if(!m)return NaN;m=m.map(Number);var f=function(x){x/=255;return x<=0.03928?x/12.92:Math.pow((x+0.055)/1.055,2.4);};return 0.2126*f(m[0])+0.7152*f(m[1])+0.0722*f(m[2]);}
    function comp(fg,bg){var a=fg.length>3?fg[3]:1;return [0,1,2].map(function(i){return Math.round(fg[i]*a+bg[i]*(1-a));});}
    function parse(c){var m=(c||'').match(/[0-9.]+/g);return m?m.map(Number):[NaN,NaN,NaN];}
    function ratio(fgStr,bgStr,surfStr){var surf=parse(surfStr||'rgb(255,255,255)');var fg=parse(fgStr),bg=parse(bgStr);var c=fg.length>3?comp(fg,surf):fg;var bc=bg.length>3?comp(bg,surf):bg;var l1=lum('rgb('+c.join(',')+')'),l2=lum('rgb('+bc.join(',')+')');var hi=Math.max(l1,l2),lo=Math.min(l1,l2);return (hi+0.05)/(lo+0.05);}
    var row=document.querySelector('.dev-row[data-id=VH-07]');
    var rowBorderless=getComputedStyle(row).borderTopWidth==='1px'&&getComputedStyle(row).backgroundColor!=='rgba(0, 0, 0, 0)';
    row.style.transition='none';row.classList.add('hover');
    var rowHoverTint=getComputedStyle(row).backgroundColor!=='rgba(0, 0, 0, 0)';
    row.classList.remove('hover');
    document.querySelector('.dev-row[data-id=DESK-03]').click();
    var selrow=document.querySelector('.dev-row[aria-selected=true]');
    var selBg=getComputedStyle(selrow).backgroundColor;
    var surf=getComputedStyle(document.body).backgroundColor;
    var selectedTintBar=selBg!=='rgba(0, 0, 0, 0)'&&getComputedStyle(selrow).borderTopColor!==getComputedStyle(row).borderTopColor;
    var tintContrast=ratio(getComputedStyle(selrow).color,selBg,surf)>=4.5;
    setTheme('dark');
    var selBgD=getComputedStyle(document.querySelector('.dev-row[aria-selected=true]')).backgroundColor;
    var tintContrastDark=ratio(getComputedStyle(document.querySelector('.dev-row[aria-selected=true]')).color,selBgD,getComputedStyle(document.body).backgroundColor)>=4.5;
    var tile=document.querySelector('#remLauncher [data-cap=control]');
    tile.style.transition='none';tile.classList.add('hover');
    var darkHoverTint=getComputedStyle(tile).backgroundColor!=='rgba(0, 0, 0, 0)';
    tile.classList.remove('hover');
    setTheme('light');
    var kinds={};document.querySelectorAll('#remGrid .dev-badges .chip').forEach(function(c){kinds[c.textContent]=1;});
    var badgeKinds=Object.keys(kinds).length<=3&&!kinds['Same account'];
    var sameAccountSubline=document.querySelector('.dev-row[data-id=DESK-03] .dev-meta .owner').textContent.includes('owner:');
    var sublineTabular=getComputedStyle(document.querySelector('.dev-row[aria-selected=true] .dev-meta')).fontVariantNumeric.includes('tabular-nums');
    var tileRealFilled=getComputedStyle(document.querySelector('#remLauncher [data-cap=control] .ic')).color===getComputedStyle(document.querySelector('.dev-row[aria-selected=true]'),'::before').backgroundColor;
    var gated=document.querySelector('#remLauncher [data-cap=file]');
    var tileGatedDashed=!!gated.querySelector('.lockcorner')&&getComputedStyle(gated).borderTopStyle!=='dashed';
    var gOp=parseFloat(getComputedStyle(gated).opacity);var gCol=getComputedStyle(gated).color.match(/[0-9.]+/g).map(Number);var gBg=getComputedStyle(document.querySelector('#remLauncher')).backgroundColor.match(/[0-9.]+/g).map(Number);var gEff=[0,1,2].map(function(i){return Math.round(gCol[i]*gOp+gBg[i]*(1-gOp));});
    var gatedContrast=gOp===1&&ratio('rgb('+gEff.join(',')+')',getComputedStyle(document.querySelector('#remLauncher')).backgroundColor,getComputedStyle(document.body).backgroundColor)>=4.5;
    var pow=document.querySelector('#remLauncher [data-pow=reboot]');pow.style.transition='none';pow.classList.add('hover');
    var powerHoverTint=getComputedStyle(pow).backgroundColor==='rgba(0, 0, 0, 0)';
    pow.classList.remove('hover');
    var lh=[...document.querySelector('.rem-listhead').children].filter(function(c){return !c.hidden;});
    var toolbarSingleRow=lh.every(function(c){return Math.abs((c.offsetTop+c.offsetHeight/2)-(lh[0].offsetTop+lh[0].offsetHeight/2))<=2;});
    var headsSentenceCase=getComputedStyle(document.querySelector('#remLauncher .dd-sec')).textTransform==='none';
    var activeEmptyHidden=document.getElementById('remActiveSec').hidden;
    var rm=false;for(var si=0;si<document.styleSheets.length;si++){try{var rs=document.styleSheets[si].cssRules;for(var ri=0;ri<rs.length;ri++){if(rs[ri].media&&rs[ri].media.mediaText.includes('prefers-reduced-motion'))rm=true;}}catch(e){}}
    var reducedMotion=rm;
    var pill=document.querySelector('#remTree .cnt');
    var pillContrast=ratio(getComputedStyle(pill).color,getComputedStyle(pill).backgroundColor,getComputedStyle(document.body).backgroundColor)>=4.5;
    return {rowBorderless,rowHoverTint,selectedTintBar,tintContrast,tintContrastDark,darkHoverTint,badgeKinds,sameAccountSubline,sublineTabular,tileRealFilled,tileGatedDashed,gatedContrast,powerHoverTint,toolbarSingleRow,headsSentenceCase,activeEmptyHidden,reducedMotion,pillContrast};`],
  ['remote-visual-tree', `goto('remote');
    var all=document.querySelector('#remTree [data-g=""]');var grp=document.querySelector('#remTree [data-g="Yard vehicles"]');
    var treeGroupIndent=parseFloat(getComputedStyle(grp).paddingLeft)-parseFloat(getComputedStyle(all).paddingLeft)>=16;
    var pill=document.querySelector('#remTree .cnt');
    var treeCountPill=getComputedStyle(pill).fontVariantNumeric.includes('tabular-nums')&&getComputedStyle(pill).borderRadius==='8px';
    var treeHeadIcons=document.getElementById('remTreeRefresh').getAttribute('aria-label')&&document.getElementById('remGroupAdd').getAttribute('aria-label')&&document.querySelectorAll('.rem-treehead button').length===2;
    grp.click();
    var okTint=getComputedStyle(document.documentElement).getPropertyValue('--ok-tint').trim();
    var selOpt=document.querySelector('#remTree [aria-selected=true]');
    var treeSelectedTint=getComputedStyle(selOpt).backgroundColor==='rgba(0, 0, 0, 0)'&&getComputedStyle(selOpt.querySelector('.tname')).fontWeight==='700'&&getComputedStyle(selOpt.querySelector('.cnt')).backgroundColor!=='rgba(0, 0, 0, 0)';
    var groupListbox=document.getElementById('remTree').getAttribute('role')==='listbox';
    return {treeGroupIndent,treeCountPill,treeHeadIcons,treeSelectedTint,groupListbox};`],
  ['remote-visual-narrow', `goto('remote');document.querySelector('.dev-row[data-id=DESK-03]').click();
    var lh=[...document.querySelector('.rem-listhead').children].filter(function(c){return !c.hidden;});
    var toolbarSingleRow=lh.every(function(c){return Math.abs((c.offsetTop+c.offsetHeight/2)-(lh[0].offsetTop+lh[0].offsetHeight/2))<=2;});
    var row=document.querySelector('#remGrid .dev-row');
    var tier3TwoLine=getComputedStyle(row.querySelector('.dev-meta')).display==='flex'&&row.getBoundingClientRect().height>44;
    var treeChipsBorderless=getComputedStyle(document.querySelector('#remTree [role=option]')).borderTopWidth==='0px';
    var narrowDegrade=getComputedStyle(document.getElementById('remListScroll')).overflowY==='visible';
    var tileDashedNarrow=!!document.querySelector('#remLauncher [data-cap=file] .lockcorner')&&getComputedStyle(document.querySelector('#remLauncher [data-cap=file]')).borderTopStyle!=='dashed';
    return {toolbarSingleRow,tier3TwoLine,treeChipsBorderless,tileDashedNarrow,narrowDegrade};`, 400],
  ['remote-left', `goto('remote');document.querySelector('.dev-row[data-id=DESK-03]').click();
    var sc=document.getElementById('remListScroll');sc.style.maxHeight='200px';
    var hr=document.querySelector('.rem-searchrow').getBoundingClientRect().top;
    var fr=document.getElementById('remLeftFoot').getBoundingClientRect().top;
    sc.scrollTop=120;
    var headerFixedSlot=Math.abs(document.querySelector('.rem-searchrow').getBoundingClientRect().top-hr)<1;
    var footerFixedSlot=Math.abs(document.getElementById('remLeftFoot').getBoundingClientRect().top-fr)<1;
    var listIndependentScroll=sc.scrollTop>0&&headerFixedSlot;
    var lf=document.getElementById('remLeftFoot').textContent;
    var footerCounts=lf.includes('All devices: '+DEVICES.length)&&lf.includes(DEVICES.filter(function(d){return d.status==='online';}).length+' online');
    var navRow=document.querySelector('#remTree [role=option][aria-selected=false]');var devRow=document.querySelector('#remGrid .dev-row');
    var navCompact=navRow.getBoundingClientRect().height<devRow.getBoundingClientRect().height;
    var navBg=getComputedStyle(document.getElementById('remTree')).backgroundColor;
    navRow.classList.add('hover');
    var navHoverDelta=getComputedStyle(navRow).backgroundColor!==navBg;
    navRow.classList.remove('hover');
    var unsel=document.querySelector('#remGrid .dev-row[aria-selected=false]');
    var rowTwoLineAlways=getComputedStyle(unsel.querySelector('.dev-meta')).display==='flex';
    var nameFullWidth=unsel.querySelector('.dev-name').getBoundingClientRect().width>150;
    var vr=ROOMS.find(function(x){return x.id==='ctrl-VH-11';});vr.estop=true;renderRemote();
    var stopRow=document.querySelector('#remGrid .dev-row[data-id=VH-11]');
    var bd=stopRow.querySelector('.dev-badges');
    var badgesSublineAlways=bd.textContent.includes('STOPPED')&&bd.scrollWidth<=bd.clientWidth+1&&Math.abs(bd.getBoundingClientRect().top-stopRow.querySelector('.metatxt').getBoundingClientRect().top)<8;
    vr.estop=false;renderRemote();
    var deg=document.querySelector('#remGrid .dot-deg');var off=document.querySelector('#remGrid .dot-off');var on=document.querySelector('#remGrid .dot-on');
    var dotThreeShapes=getComputedStyle(deg).backgroundImage.includes('linear-gradient')&&getComputedStyle(off).backgroundColor==='rgba(0, 0, 0, 0)'&&getComputedStyle(on).backgroundColor!=='rgba(0, 0, 0, 0)';
    function lum(c){var m=(c||'').match(/[0-9.]+/g);if(!m)return NaN;m=m.map(Number);var f=function(x){x/=255;return x<=0.03928?x/12.92:Math.pow((x+0.055)/1.055,2.4);};return 0.2126*f(m[0])+0.7152*f(m[1])+0.0722*f(m[2]);}
    function ratio(a,b){var l1=lum(a),l2=lum(b);var hi=Math.max(l1,l2),lo=Math.min(l1,l2);return (hi+0.05)/(lo+0.05);}
    var dotOffContrast=ratio(getComputedStyle(off).borderTopColor,getComputedStyle(document.querySelector('.rem-lobby-left')).backgroundColor)>=3;
    var osMonochrome=getComputedStyle(document.querySelector('#remGrid .os')).color===getComputedStyle(document.querySelector('#remGrid .dev-meta')).color;
    var noTriangle=!document.querySelector('#remGrid .g-warn')&&!!on&&!!off&&!!deg;
    document.querySelector('#remTree [data-g="Yard vehicles"]').click();
    var so=document.querySelector('#remTree [aria-selected=true]');
    var groupSelBoldNoTint=getComputedStyle(so).backgroundColor==='rgba(0, 0, 0, 0)'&&getComputedStyle(so.querySelector('.tname')).fontWeight==='700';
    var groupPillAccent=getComputedStyle(so.querySelector('.cnt')).backgroundColor!=='rgba(0, 0, 0, 0)';
    document.querySelector('#remTree [data-g=""]').click();
    var selrow=document.querySelector('#remGrid .dev-row[aria-selected=true]');
    var deviceSelTintBar=getComputedStyle(selrow).backgroundColor!=='rgba(0, 0, 0, 0)'&&getComputedStyle(selrow,'::before').width==='2px';
    var se=document.getElementById('remSearch');se.value='zzz';se.oninput();
    var emp=document.getElementById('remDevEmpty');
    var emptyCentered=!emp.hidden&&sc.classList.contains('is-empty');
    se.value='';se.oninput();
    applyProto('sim','remoteerror');
    var rt=document.getElementById('remRetry');var rr=rt.getBoundingClientRect();
    var el=document.elementFromPoint(rr.left+rr.width/2,rr.top+rr.height/2);
    var errorRetryReachable=!rt.hidden&&!!el&&(el===rt||rt.contains(el));
    rt.click();remLoading=false;renderRemote();
    var fr2=document.querySelector('#remGrid .dev-row[aria-selected=true]');fr2.focus();
    fr2.dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowDown',bubbles:true,cancelable:true}));
    var rowRoving=document.activeElement.classList.contains('dev-row')&&document.activeElement!==fr2&&document.activeElement.tabIndex===0;
    sc.style.maxHeight='';
    return {headerFixedSlot,footerFixedSlot,listIndependentScroll,footerCounts,navCompact,navHoverDelta,rowTwoLineAlways,nameFullWidth,badgesSublineAlways,dotThreeShapes,dotOffContrast,osMonochrome,noTriangle,groupSelBoldNoTint,groupPillAccent,deviceSelTintBar,emptyCentered,errorRetryReachable,rowRoving};`],
  ['remote-left-scroll', `goto('remote');
    var sc=document.getElementById('remListScroll');sc.style.maxHeight='200px';
    var hr=document.querySelector('.rem-searchrow').getBoundingClientRect().top;
    sc.scrollTop=120;
    var headerFixedSlot=Math.abs(document.querySelector('.rem-searchrow').getBoundingClientRect().top-hr)<1;
    var listIndependentScroll=sc.scrollTop>0;
    var fr=document.getElementById('remLeftFoot').getBoundingClientRect().top;
    var footerFixedSlot=Math.abs(document.getElementById('remLeftFoot').getBoundingClientRect().top-fr)<1;
    sc.style.maxHeight='';
    return {headerFixedSlot,listIndependentScroll,footerFixedSlot};`],
  ['remote-tools', `goto('remote');
    var fb=document.getElementById('remFilterBtn');fb.click();
    var pop=document.getElementById('remFilterPop');
    var pr=pop.getBoundingClientRect();var br=fb.getBoundingClientRect();
    var filterPopAnchored=!pop.hidden&&pr.top>=br.bottom-2&&pr.left>=0&&pr.right<=innerWidth;
    document.querySelector('#remFilters [data-f=online]').click();
    var filterBadgeCount=document.getElementById('remFilterCount').textContent==='1'&&!document.getElementById('remFilterCount').hidden;
    document.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true,cancelable:true}));
    var filterEscCloses=pop.hidden&&fb.getAttribute('aria-expanded')==='false';
    var noTitleTooltips=!fb.getAttribute('title')&&!document.getElementById('remSortBtn').getAttribute('title')&&!document.getElementById('remTreeRefresh').getAttribute('title')&&!document.getElementById('remGroupAdd').getAttribute('title');
    var placeholderShort=document.getElementById('remSearch').placeholder==='Search devices';
    var navFlat=getComputedStyle(document.getElementById('remTree')).backgroundColor==='rgba(0, 0, 0, 0)';
    var lh=[...document.querySelector('.rem-listhead').children].filter(function(c){return !c.hidden;});
    var mergedSingleRow=lh.every(function(c){return Math.abs((c.offsetTop+c.offsetHeight/2)-(lh[0].offsetTop+lh[0].offsetHeight/2))<=2;});
    return {filterPopAnchored,filterBadgeCount,filterEscCloses,noTitleTooltips,placeholderShort,navFlat,mergedSingleRow};`],
  ['remote-row', `goto('remote');document.querySelector('.dev-row[data-id=DESK-03]').click();
    var row=document.querySelector('#remGrid .dev-row[data-id=VH-07]');
    var nm=row.querySelector('.dev-name').getBoundingClientRect().left;
    var mt=row.querySelector('.metatxt').getBoundingClientRect().left;
    var sublineAlign=Math.abs(nm-mt)<=2;
    var txt=row.querySelector('.metatxt').textContent;
    var sublineOrder=txt.indexOf('linux')<txt.indexOf('10.20.1.7')&&txt.indexOf('online')===0;
    var selfChip=document.querySelector('#remGrid .dev-badges .self');
    var barColor=getComputedStyle(document.querySelector('#remGrid .dev-row[aria-selected=true]'),'::before').backgroundColor;
    var badgeAccent=selfChip&&getComputedStyle(selfChip).borderTopColor===barColor;
    var go=row.querySelector('.dev-go');
    var hidden0=getComputedStyle(go).opacity==='1';
    go.style.transition='none';row.classList.add('hover');
    var hoverChevron=hidden0&&getComputedStyle(go).borderTopWidth==='1px';
    row.classList.remove('hover');
    var coarseHide=false;for(var si=0;si<document.styleSheets.length;si++){try{var rs=document.styleSheets[si].cssRules;for(var ri=0;ri<rs.length;ri++){if(rs[ri].media&&rs[ri].media.mediaText.includes('coarse')&&rs[ri].cssText.includes('.rowgo'))coarseHide=true;}}catch(e){}}
    var coarseChevronHide=coarseHide;
    return {sublineAlign,sublineOrder,badgeAccent,hoverChevron,coarseChevronHide};`],
  ['remote-launcher-visual', `goto('remote');document.querySelector('.dev-row[data-id=DESK-03]').click();
    var L=document.getElementById('remLauncher');
    L.style.flex='0 0 360px';
    var containerTwoCol=getComputedStyle(document.querySelector('#remLauncher .rem-tiles:not(.rem-power)')).gridTemplateColumns.split(' ').length===2;
    var midBandPower3=getComputedStyle(document.querySelector('#remLauncher .rem-power')).gridTemplateColumns.split(' ').length===3;
    L.style.flex='0 0 300px';
    var powerVertical=getComputedStyle(document.querySelector('#remLauncher .rem-power')).gridTemplateColumns.split(' ').length===1;
    L.style.flex='0 0 397px';
    var labelNoWrap=[...L.querySelectorAll('.rem-tiles:not(.rem-power) .rem-tile')].every(function(t){return t.scrollWidth<=t.clientWidth;});
    L.style.flex='0 0 342px';
    var powerNoOverflow=[...L.querySelectorAll('.rem-power .rem-tile')].every(function(t){return t.scrollWidth<=t.clientWidth;});
    L.style.flex='';
    var powerThreeCol=getComputedStyle(document.querySelector('#remLauncher .rem-power')).gridTemplateColumns.split(' ').length===3;
    var borderlessDivider=getComputedStyle(L).borderLeftWidth==='1px'&&getComputedStyle(L).borderTopWidth==='0px';
    function lum(c){var m=(c||'').match(/[0-9.]+/g);if(!m)return NaN;m=m.map(Number);var f=function(x){x/=255;return x<=0.03928?x/12.92:Math.pow((x+0.055)/1.055,2.4);};return 0.2126*f(m[0])+0.7152*f(m[1])+0.0722*f(m[2]);}
    function ratio(a,b){var l1=lum(a),l2=lum(b);return (Math.max(l1,l2)+0.05)/(Math.min(l1,l2)+0.05);}
    var sep=getComputedStyle(L).borderLeftColor;
    var sepContrast=ratio(sep,getComputedStyle(document.body).backgroundColor)>=3;
    setTheme('dark');
    var sepContrastDark=ratio(getComputedStyle(L).borderLeftColor,getComputedStyle(document.body).backgroundColor)>=3;
    setTheme('light');
    var dd=DEVICES.find(function(x){return x.id==='DESK-03';});dd.revoked=true;renderLauncher();
    var ban=L.querySelector('[role=alert]');
    var errorBanner=!!ban&&getComputedStyle(ban).backgroundColor!=='rgba(0, 0, 0, 0)'&&getComputedStyle(ban).color===getComputedStyle(document.body).color;
    dd.revoked=false;renderLauncher();
    remLoading=true;renderRemote();
    var loadingSkeleton=!!L.querySelector('.lm-skel')&&L.textContent.includes('Loading device capabilities');
    remLoading=false;renderRemote();
    remSel=null;renderRemote();
    var E=document.getElementById('remLauncherEmpty');
    var emptyTopAligned=!E.hidden&&Math.abs(E.getBoundingClientRect().top-document.querySelector('.rem-lobby-wrap').getBoundingClientRect().top)<4;
    document.querySelector('.dev-row[data-id=DESK-03]').click();
    var rn=document.getElementById('lmRename'),inf=document.getElementById('lmInfo'),cp=document.getElementById('lmCopy');
    var t1=document.querySelector('#remLauncher [data-cap]'),p1=document.querySelector('#remLauncher [data-pow]');
    var focusOrder=!!(rn.compareDocumentPosition(inf)&Node.DOCUMENT_POSITION_FOLLOWING)&&!!(inf.compareDocumentPosition(cp)&Node.DOCUMENT_POSITION_FOLLOWING)&&!!(cp.compareDocumentPosition(t1)&Node.DOCUMENT_POSITION_FOLLOWING)&&!!(t1.compareDocumentPosition(p1)&Node.DOCUMENT_POSITION_FOLLOWING);
    var iconAriaLabels=!!rn.getAttribute('aria-label')&&!!cp.getAttribute('aria-label');
    return {containerTwoCol,midBandPower3,labelNoWrap,powerNoOverflow,powerVertical,powerThreeCol,borderlessDivider,sepContrast,sepContrastDark,errorBanner,loadingSkeleton,emptyTopAligned,focusOrder,iconAriaLabels};`],
  ['remote-chrome', `myRole='admin';goto('remote');document.querySelector('.dev-row[data-id=DESK-03]').click();document.querySelector('#remLauncher [data-cap=control]').click();
    await new Promise(function(r){setTimeout(r,750);});
    var sb=document.querySelector('#remSafeBar .inner');
    function alpha(c){var m=c.match(/[0-9.]+/g);return m&&m.length>3?parseFloat(m[3]):1;}
    var safeBarScrim=alpha(getComputedStyle(sb).backgroundColor)===0.85&&getComputedStyle(sb).color==='rgb(231, 236, 245)';
    setTheme('dark');
    var safeBarScrimDark=alpha(getComputedStyle(sb).backgroundColor)===0.85&&getComputedStyle(sb).color==='rgb(231, 236, 245)';
    setTheme('light');
    var stage=document.getElementById('remStage').getBoundingClientRect();
    var r=sb.getBoundingClientRect();
    var safeBarCoverPct=(r.width*r.height)/(stage.width*stage.height)<=0.06;
    var es=document.getElementById('remEstop');
    var estopOpaqueBase=alpha(getComputedStyle(es).backgroundColor)===0.85;
    function lum(c){var m=(c||'').match(/[0-9.]+/g);if(!m)return NaN;m=m.map(Number);var f=function(x){x/=255;return x<=0.03928?x/12.92:Math.pow((x+0.055)/1.055,2.4);};return 0.2126*f(m[0])+0.7152*f(m[1])+0.0722*f(m[2]);}
    function ratio(a,b){var l1=lum(a),l2=lum(b);return (Math.max(l1,l2)+0.05)/(Math.min(l1,l2)+0.05);}
    function compParse(biStr,bgStr){var m=(biStr||'').match(/rgba?\([^)]*\)/g)||[];var col=(bgStr.match(/[0-9.]+/g)||[]).map(Number).slice(0,3);if(m.length){var p=m[0].match(/[0-9.]+/g).map(Number);var a=p.length>3?p[3]:1;col=[0,1,2].map(function(k){return Math.round(p[k]*a+col[k]*(1-a));});}return 'rgb('+col.join(',')+')';}
    function compStops(el){return compParse(getComputedStyle(el).backgroundImage,getComputedStyle(el).backgroundColor);}
    var estopInkContrast=ratio(getComputedStyle(es).color,compStops(es))>=4.5;
    setTheme('dark');
    var estopInkContrastDark=ratio(getComputedStyle(es).color,compStops(es))>=4.5;
    setTheme('light');
    var grad='linear-gradient(rgba(255, 90, 95, 0.16), rgba(255, 90, 95, 0.16)), none';
    var compositeSelfTest=compParse(grad,'rgb(28, 39, 64)')==='rgb(64,47,69)'&&compParse(grad,'rgb(14, 21, 38)')==='rgb(53,32,47)';
    var ballSessionVisible=getComputedStyle(document.getElementById('remBall')).display==='flex';
    var connectingGuard=(renderRemHud.toString().match(/if\\(!remConnecting\\)/g)||[]).length>=2;

    var corner=document.getElementById('remSimCorner');
    document.body.classList.add('rem-hideov');
    var simCornerNeverHide=getComputedStyle(corner).display!=='none';
    document.body.classList.remove('rem-hideov');
    var mock=document.querySelector('#remTile .sim-desktop');
    var mockNonInteractive=getComputedStyle(mock).pointerEvents==='none'&&!mock.querySelector('button,input,[tabindex]');
    var escChipShort=document.getElementById('remEscHint').textContent==='Esc';
    var pairVisible=getComputedStyle(document.getElementById('hudLat')).display!=='none'&&getComputedStyle(document.getElementById('hudJit')).display!=='none';
    var stageScrim=getComputedStyle(document.getElementById('remStage')).backgroundColor===getComputedStyle(document.getElementById('remTile')).backgroundColor;
    remCapture();
    var estopCtxCaptured=getComputedStyle(es).display==='flex';
    remCaptured=false;renderRemState();
    var estopCtxViewHidden=getComputedStyle(es).display==='none';
    remCaptured=true;renderRemState();
    var estopContextual=estopCtxCaptured&&estopCtxViewHidden&&getComputedStyle(es).display==='flex';
    var estopUnified=Math.round(es.getBoundingClientRect().height)===30&&getComputedStyle(es).borderTopWidth==='1px'&&getComputedStyle(document.getElementById('remExpand')).borderTopWidth==='0px';
    var stg2=document.getElementById('remStage').getBoundingClientRect();
    var sessionFullBleed=stg2.top<=1&&stg2.left<=1&&stg2.right>=innerWidth-1&&stg2.bottom>=innerHeight-1;
    function fourSides(el){var c=getComputedStyle(el);return c.borderTopWidth==='0px'&&c.borderRightWidth==='0px'&&c.borderBottomWidth==='0px'&&c.borderLeftWidth==='0px';}
    var barBorderless=fourSides(sb);
    var chipBorderless=[...sb.querySelectorAll('.chip')].filter(function(c){return c.id!=='remStateChip';}).every(fourSides);
    var stateChipFramed=getComputedStyle(document.getElementById('remStateChip')).borderTopWidth==='1px';
    var expandGhost=fourSides(document.getElementById('remExpand'))&&getComputedStyle(document.getElementById('remExpand')).color==='rgb(142, 163, 249)';
    var escComp=ratio(getComputedStyle(document.getElementById('remEscHint')).color,compStops(sb));
    setTheme('dark');
    var escCompDark=ratio(getComputedStyle(document.getElementById('remEscHint')).color,compStops(sb));
    setTheme('light');
    var escInkContrast=escComp>=4.5&&escCompDark>=4.5;
    es.dispatchEvent(new PointerEvent('pointerdown',{bubbles:true,cancelable:true,pointerId:9,button:0,isPrimary:true}));
    await new Promise(function(r){setTimeout(r,220);});
    var holdTextMirror=document.getElementById('remEstopHoldTxt').textContent.includes('Hold');
    es.dispatchEvent(new PointerEvent('pointerup',{bubbles:true,cancelable:true,pointerId:9}));
    return {safeBarScrim,safeBarScrimDark,safeBarCoverPct,estopOpaqueBase,estopInkContrast,estopInkContrastDark,compositeSelfTest,ballSessionVisible,connectingGuard,simCornerNeverHide,mockNonInteractive,escChipShort,pairVisible,holdTextMirror,stageScrim,sessionFullBleed,barBorderless,chipBorderless,stateChipFramed,expandGhost,escInkContrast,estopContextual,estopUnified};`],
  ['remote-menu', `myRole='admin';goto('remote');document.querySelector('.dev-row[data-id=DESK-03]').click();document.querySelector('#remLauncher [data-cap=control]').click();
    await new Promise(function(r){setTimeout(r,750);});
    remCapture();document.getElementById('remBall').click();
    var pop=document.getElementById('remBallPop');
    var stripGated=['bmCast','bmVol','bmMin'].every(function(id){return document.getElementById(id).getAttribute('aria-disabled')==='true';})&&document.getElementById('remFs3').getAttribute('aria-disabled')!=='true';
    var tilesGated=['bmMute','bmPrivacy'].every(function(id){var t=document.getElementById(id);return t.getAttribute('aria-disabled')==='true'&&t.querySelector('.bm-check').hasAttribute('hidden');});
    var order=['bmMode','remSecRow','remInputRow','remDisplayRow','bmFile','bmPeriph','bmInteract','remCopyInfo','remBack3'].map(function(id){return document.getElementById(id);});
    var rowsOrder=order.every(function(el,i){return i===0|| (order[i-1].compareDocumentPosition(el)&Node.DOCUMENT_POSITION_FOLLOWING);});
    var rowsBorderless=order.every(function(el){return getComputedStyle(el).borderTopWidth==='0px';});
    document.getElementById('remDisplayRow').click();
    var dp=document.getElementById('remDisplayPop');
    var displayRows=['gQuality','gRefresh','gRes','remFitRow','bmMulti','bmTrue','bmExtreme','remAdvRow'].every(function(id){return !!dp.querySelector('#'+id);});
    var togglesGated=['bmMulti','bmTrue','bmExtreme'].every(function(id){return document.getElementById(id).getAttribute('aria-disabled')==='true';});
    document.getElementById('bmTrue').click();
    var gatedNeverToggles=document.getElementById('bmTrue').getAttribute('aria-checked')==='false'&&document.getElementById('guardTip').textContent.includes('Not available yet');
    document.getElementById('remFitRow').click();
    var fitReal=!document.getElementById('remZoomFit').getAttribute('aria-disabled')&&!document.getElementById('remZoomOrig').getAttribute('aria-disabled');
    document.getElementById('remAdvRow').click();
    var advReal=!document.getElementById('remOsdBtn3').getAttribute('aria-disabled')&&!document.getElementById('remInfoBtn3').getAttribute('aria-disabled')&&document.getElementById('gEncoder').getAttribute('aria-disabled')==='true';
    document.getElementById('remInputRow').click();
    var controlReal=!!document.getElementById('remSessRow2')&&!!document.getElementById('remKeysRow2')&&!document.getElementById('remSessRow2').getAttribute('aria-disabled');
    var r=remRoom();r.estop=true;updateBmBadge();
    var badgeDynamic=!document.getElementById('bmSafeBadge').hidden&&document.getElementById('bmSafeBadge').textContent==='1';
    r.estop=false;updateBmBadge();
    document.getElementById('remDisplayRow').click();
    var parentStays=!pop.hidden&&!document.getElementById('remDisplayPop').hidden;
    var childRight=document.getElementById('remDisplayPop').getBoundingClientRect().left>=pop.getBoundingClientRect().right-1;
    document.getElementById('remFitRow').click();
    var thirdRight=document.getElementById('remFitPop').getBoundingClientRect().left>=document.getElementById('remDisplayPop').getBoundingClientRect().right-1;
    document.getElementById('remInputRow').dispatchEvent(new PointerEvent('pointerover',{bubbles:true}));
    await new Promise(function(r){setTimeout(r,200);});
    var hoverOpens=!document.getElementById('remInputPop').hidden;
    document.getElementById('remDisplayRow').focus();
    document.getElementById('remDisplayRow').dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowRight',bubbles:true,cancelable:true}));
    var kbRight=!document.getElementById('remDisplayPop').hidden;
    document.activeElement.dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowLeft',bubbles:true,cancelable:true}));
    var kbLeft=document.getElementById('remDisplayPop').hidden;
    document.getElementById('remDisplayRow').click();document.getElementById('remFitRow').click();
    document.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true,cancelable:true}));
    var escLayered=document.getElementById('remFitPop').hidden&&!document.getElementById('remDisplayPop').hidden;
    document.getElementById('remDisplayRow').dispatchEvent(new PointerEvent('pointerover',{bubbles:true}));
    document.getElementById('bmFile').dispatchEvent(new PointerEvent('pointerover',{bubbles:true}));
    document.getElementById('gQuality').dispatchEvent(new PointerEvent('pointerover',{bubbles:true}));
    var siblingCrossKeepsChild=!document.getElementById('remDisplayPop').hidden;
    document.getElementById('bmFile').dispatchEvent(new PointerEvent('pointerover',{bubbles:true}));
    await new Promise(function(r){setTimeout(r,400);});
    var graceExpires=document.getElementById('remDisplayPop').hidden;
    var dr=document.getElementById('remDisplayRow').getBoundingClientRect();
    document.dispatchEvent(new PointerEvent('pointermove',{bubbles:true,clientX:dr.right-4,clientY:dr.top+dr.height/2}));
    document.getElementById('remDisplayRow').dispatchEvent(new PointerEvent('pointerover',{bubbles:true}));
    var dp2=document.getElementById('remDisplayPop').getBoundingClientRect();
    document.dispatchEvent(new PointerEvent('pointermove',{bubbles:true,clientX:(dr.right+dp2.left)/2,clientY:dr.top+dr.height/2}));
    document.getElementById('bmFile').dispatchEvent(new PointerEvent('pointerover',{bubbles:true}));
    var triangleSuppress=!document.getElementById('remDisplayPop').hidden;
    var ir=document.getElementById('remInputRow').getBoundingClientRect();
    document.dispatchEvent(new PointerEvent('pointermove',{bubbles:true,clientX:ir.left+ir.width/2,clientY:ir.top+ir.height/2}));
    document.getElementById('remInputRow').dispatchEvent(new PointerEvent('pointerover',{bubbles:true}));
    var intentNotImmediate=document.getElementById('remInputPop').hidden;
    await new Promise(function(r){setTimeout(r,200);});
    var openIntentDelay=intentNotImmediate&&!document.getElementById('remInputPop').hidden;
    document.getElementById('remSecRow').click();
    var clickImmediate=!document.getElementById('remSecPop').hidden;
    myRole='viewer';updateBmBadge();
    var forceAdminGate=document.getElementById('bmForce').getAttribute('aria-disabled')==='true';
    document.getElementById('bmForce').click();
    var forceGateNoDlg=!document.querySelector('.modal-scrim.show');
    myRole='admin';updateBmBadge();
    remRoom().holder='dispatch.x';
    document.getElementById('bmForce').click();
    var forceDlg=!!document.querySelector('.modal-scrim.show');
    var cf=document.getElementById('cfmOk');if(cf)cf.click();
    var forceClickExecutes=forceDlg&&remRoom().holder===null;
    document.getElementById('remBack3').click();
    var endReleases=document.getElementById('remLobby').hidden===false&&remCaptured===false;
    return {stripGated,tilesGated,rowsOrder,rowsBorderless,displayRows,togglesGated,gatedNeverToggles,fitReal,advReal,controlReal,badgeDynamic,parentStays,childRight,thirdRight,hoverOpens,kbRight,kbLeft,escLayered,siblingCrossKeepsChild,graceExpires,triangleSuppress,openIntentDelay,clickImmediate,forceAdminGate,forceGateNoDlg,forceClickExecutes,endReleases};`],
  ['remote-bar-stable', `myRole='admin';goto('remote');document.querySelector('.dev-row[data-id=DESK-03]').click();document.querySelector('#remLauncher [data-cap=control]').click();
    await new Promise(function(r){setTimeout(r,750);});
    var sb=document.querySelector('#remSafeBar .inner');
    function rect(){var q=sb.getBoundingClientRect();return Math.round(q.width)+'x'+Math.round(q.height);}
    var r0=rect();var stable=true;
    var r=remRoom();
    remKeys.add('w');remInput();remKeys.clear();remLeaseTtl=0;leaseTick();stable=stable&&rect()===r0;
    remInputRate=1234;leaseTick();stable=stable&&rect()===r0;
    document.getElementById('remClock').textContent='00:00:00';stable=stable&&rect()===r0;
    remCapture();stable=stable&&rect()===r0;
    remLatch();stable=stable&&rect()===r0;
    document.getElementById('remRelease').hidden=false;stable=stable&&rect()===r0;
    remEstopCancel();r.estop=false;r.lease.state='idle';remInputRate=0;leaseTick();
    var ir=sb.getBoundingClientRect();
    var slotsInsideBar=[...sb.querySelectorAll('.chip,.opswap,#remExpand,#remEscHint')].filter(function(el){return el.getBoundingClientRect().width>0;}).every(function(el){var q=el.getBoundingClientRect();return q.left>=ir.left-1&&q.right<=ir.right+1&&q.top>=ir.top-1&&q.bottom<=ir.bottom+1;});
    var relPos=false,relShow=false;for(var si=0;si<document.styleSheets.length;si++){try{var rs=document.styleSheets[si].cssRules;for(var ri=0;ri<rs.length;ri++){var sel=rs[ri].selectorText||'';if(sel.indexOf('#remReleased')>=0&&rs[ri].style&&rs[ri].style.position==='absolute')relPos=true;if(sel==='#remReleased.show'&&rs[ri].style.display==='block')relShow=true;}}catch(e){}}
    var stackCol=false,stackAbs=false;for(var sj=0;sj<document.styleSheets.length;sj++){try{var rr3=document.styleSheets[sj].cssRules;for(var rj=0;rj<rr3.length;rj++){if((rr3[rj].selectorText||'')==='#remBannerStack'){stackCol=rr3[rj].style.flexDirection==='column';stackAbs=rr3[rj].style.position==='absolute';}}}catch(e){}}
    var bannerStackNoOverlap=stackCol&&stackAbs;
    var releasedOutOfFlow=stackAbs&&stackCol&&relShow;
    var tk2=document.getElementById('remTake'),ex2=document.getElementById('remExpand');
    var btnNoWrap=tk2.scrollWidth<=tk2.clientWidth&&ex2.scrollWidth<=ex2.clientWidth&&Math.round(tk2.getBoundingClientRect().height)===30&&Math.round(ex2.getBoundingClientRect().height)===30;
    return {barStable:stable,slotsInsideBar,releasedOutOfFlow,bannerStackNoOverlap,btnNoWrap};`],
['accounts-narrow', `myRole='admin';goto('accounts');
    var noOv=document.documentElement.scrollWidth<=innerWidth;
    var card=getComputedStyle(document.querySelector('#accBody td:nth-child(2)')).display==='flex';
    var tailHidden=getComputedStyle(document.querySelector('#modeInd .mi-tail')).display==='none';
    return {noOv,card,tailHidden};`, 600],
  ['admin-controls', `var ids=['devices','rooms','sessions','streams','config','keys','audit','settings','signaling'];var bad=[];
    ids.forEach(id=>{goto(id);document.querySelectorAll('#page-'+id+' .btn,#page-'+id+' .chip-btn,#page-'+id+' .seg,#page-'+id+' .search,#page-'+id+' .iconbtn,#page-'+id+' .msg-in').forEach(e=>{if(!e.getClientRects().length)return;var h=Math.round(e.getBoundingClientRect().height);if(h!==30)bad.push(id+':'+h);});});
    var h30=bad.length===0;
    goto('devices');var s0=Math.round(document.querySelector('#devSeg').getBoundingClientRect().height);
    document.querySelector('#devSeg [data-f=pending]').click();var s1=Math.round(document.querySelector('#devSeg').getBoundingClientRect().height);
    var stateStable=s0===30&&s1===30;
    var aw=[...document.querySelectorAll('#page-devices .tbl .actions .btn')].map(b=>Math.round(b.getBoundingClientRect().width));
    var actionsUniform=aw.length>0&&new Set(aw).size===1&&aw[0]===72;
    return {h30,stateStable,actionsUniform};`],
  ['admin-spoke', `goto('admin');document.querySelector('#adminHub [data-go=devices]').click();
    var back=!!document.querySelector('#page-devices [data-back]');
    var tw=document.querySelector('#page-devices .tbl-wrap');var noHScroll=tw.scrollWidth<=tw.clientWidth;
    var card=getComputedStyle(document.querySelector('#page-devices .tbl tbody td:nth-child(2)')).display==='flex';
    var crumbVisible=getComputedStyle(document.getElementById('crumb')).display!=='none';
    document.querySelector('#page-devices [data-back]').click();var backWorks=current==='admin';
    return {back,noHScroll,card,crumbVisible,backWorks};`, 600],
  ['board-1100', BOARD, 1100],
  ['board-1000', BOARD, 1000],
  ['board-1200', BOARD, 1200],
];

/* C65 negative self-check: sabotage that MUST turn the case red (run with --negative). */
const NEG = {
  floor: `autoGrant=function(){return false;};`,
  queue: `FLOOR_CFG.queueMax=99;`,
  'board-1100': `var st=document.createElement('style');st.textContent='.row-actions{display:flex;flex-wrap:wrap;width:120px}';document.head.appendChild(st);`,
  evidence: `evPurge=function(){};`,
  'device-drawer': `openDevDrawer=function(){};`,
  'device-config': `flushStaged=function(){};`,
  'device-role': `setGuard=function(){};`,
  accounts: `renderAccounts=function(){};`,
  psk: `doRotate=function(){};`,
  creds: `resetSecret=function(){};`,
  fleet: `renderFleetStrip=function(){};`,
  'monitor-link': `renderAvail=function(){};`,
  'audit-ring': `renderAudit=function(){};`,
  'meet-lobby': `renderMeeting=function(){};`,
  'meet-prejoin': `openPrejoin=function(){};`,
  'meet-live': `holdTick=function(){};`,
  'meet-host': `renderRoster=function(){};`,
  'meet-state': `renderMeetPhase=function(){};`,
  'meet-exit': `holdTick=function(){};`,
  'meet-polish': `speakerTick=function(){};`,
  'meet-polish-narrow': `openMeetPanel=function(){};`,
  'remote-lobby': `renderRemote=function(){};`,
  'remote-lease': `leaseTick=function(){};`,
  'remote-timing': `leaseTick=function(){};`,
  'remote-estop': `estopHoldTick=function(){};`,
  'remote-handover': `acceptHandover=function(){};`,
  'remote-input': `gamepadTick=function(){};`,
  'remote-fixes': `leaseTick=function(){};`,
  'remote-layout': `renderRemSession=function(){};`,
  'remote-layout-narrow': `var st=document.createElement('style');st.textContent='#remSafeBar .inner{flex-wrap:nowrap}';document.head.appendChild(st);`,
  'settings-login': `renderSettings=function(){};`,
  'settings-index': `renderConfigIndex=function(){};`,
  'remote-safetyfix': `remInput=function(){};`,
  'remote-window': `ballDockTick=function(){};`,
  'remote-launcher': `renderLauncher=function(){};`,
  'remote-launcher-narrow': `renderLauncher=function(){};`,
  'remote-devlist': `renderDevRows=function(){};`,
  'remote-devlist-narrow': `renderDevRows=function(){};`,
  'remote-devlist-1600': `renderDevRows=function(){};`,
  'tower': `renderTower=function(){};`,
  'tower-narrow': `renderTower=function(){};`,
  'intercom-space': `pttDown=function(){};`,
  'remote-visual': `renderDevRows=function(){};`,
  'remote-visual-tree': `renderDevTree=function(){};`,
  'remote-visual-narrow': `renderLauncher=function(){};`,
  'remote-left': `renderDevRows=function(){};`,
  'remote-left-scroll': `document.getElementById('remListScroll').style.overflow='visible';document.querySelector('.rem-lobby-left').style.height='auto';`,
  'remote-tools': `renderFilterChips=function(){};`,
  'remote-row': `renderDevRows=function(){};`,
  'remote-unify': `renderDevRows=function(){};`,
  'remote-launcher-visual': `renderLauncher=function(){};`,
  'remote-chrome': `renderRemSession=function(){};`,
  'remote-bar-stable': `var st=document.createElement('style');st.textContent='#remSafeBar .chip{width:auto!important}';document.head.appendChild(st);`,
  'remote-menu': `confirmDlg=function(){};`,
  'nav-collapse': `syncNavCollapse=function(){};`,
  'accounts-narrow': `renderAccounts=function(){};`,
  notif: `renderNotif=function(){};`,
};

function serve() {
  return new Promise(res => {
    const srv = http.createServer((req, rq) => {
      const f = path.join(PROTO, req.url === '/' ? 'portal-unified.html' : req.url.split('?')[0]);
      fs.readFile(f, (e, d) => { if (e) { rq.statusCode = 404; rq.end(); } else { rq.setHeader('content-type', 'text/html'); rq.end(d); } });
    });
    srv.listen(PORT, '127.0.0.1', () => res(srv));
  });
}

class Marionette {
  constructor(sock) { this.sock = sock; this.buf = ''; this.id = 0; this.pending = new Map();
    sock.on('data', d => { this.buf += d.toString(); this.drain(); }); }
  drain() { for (;;) { const i = this.buf.indexOf(':'); if (i < 0) return;
      const len = parseInt(this.buf.slice(0, i), 10); if (isNaN(len)) { this.buf = ''; return; }
      if (this.buf.length < i + 1 + len) return;
      const msg = JSON.parse(this.buf.slice(i + 1, i + 1 + len)); this.buf = this.buf.slice(i + 1 + len);
      if (Array.isArray(msg) && msg[0] === 1) { const p = this.pending.get(msg[1]); if (p) { this.pending.delete(msg[1]); p(msg[2], msg[3]); } } } }
  cmd(name, params = {}) { const id = ++this.id; const payload = JSON.stringify([0, id, name, params]);
    return new Promise((res, rej) => { this.pending.set(id, (err, r) => err ? rej(new Error(JSON.stringify(err))) : res(r));
      this.sock.write(payload.length + ':' + payload); }); }
}

async function main() {
  const srv = await serve();
  const prof = fs.mkdtempSync(path.join(os.tmpdir(), 'harness-prof-'));
  const ff = spawn('firefox', ['--headless', '--marionette', '--profile', prof, '--no-remote', 'about:blank'], { stdio: 'ignore' });
  process.on('exit', () => { try { ff.kill(); } catch (e) {} });
  await new Promise(r => setTimeout(r, 4000));
  const sock = await new Promise((res, rej) => { const s = net.connect(2828, '127.0.0.1', () => res(s)); s.on('error', rej); });
  await new Promise(r => sock.once('data', r)); // handshake
  const m = new Marionette(sock);
  await m.cmd('WebDriver:NewSession', {});
  const results = {}; let fail = 0;
  const NEG_MODE = process.argv.includes('--negative');
  let CURW = 1440;
  const run = async (src) => {
    const isAsync = /\bawait\b/.test(src);
    const inner = isAsync ? 'try{(async function(){' + src + '})().then(function(v){window.__result=v;},function(e){window.__result={__err:String(e && e.message || e)+" @"+String(e && e.stack || "").slice(0,220)};});}catch(e){window.__result={__err:String(e && e.message || e)+" @"+String(e && e.stack || "").slice(0,220)};}' : 'try{window.__result=(function(){' + src + '})();}catch(e){window.__result={__err:String(e && e.message || e)+" @"+String(e && e.stack || "").slice(0,220)};}';
    const wrapped = "var s=document.createElement('script');s.textContent=" + JSON.stringify(inner) + ";document.documentElement.appendChild(s);var r=window.__result;window.__result=undefined;return r;";
    const exec = async () => { let r = await m.cmd('WebDriver:ExecuteScript', { script: wrapped, args: [] }); let v = (r && typeof r === 'object' && 'value' in r) ? r.value : r; if (v === undefined || v === null) { for (let i = 0; i < 80; i++) { await new Promise(rs => setTimeout(rs, 100)); const p = await m.cmd('WebDriver:ExecuteScript', { script: 'var q=window.__result;window.__result=undefined;return q;', args: [] }); v = (p && typeof p === 'object' && 'value' in p) ? p.value : p; if (v !== undefined && v !== null) break; } } return { value: v }; };
    const withTimeout = (p, ms) => Promise.race([p, new Promise((_, rej) => setTimeout(() => rej(new Error('execute-timeout')), ms))]);
    const unwrap = r => ({ val: (r && typeof r === 'object' && 'value' in r) ? r.value : r, err: null });
    try { return unwrap(await withTimeout(exec(), 25000)); }
    catch (e) {
      if (String(e.message) !== 'execute-timeout') return { val: {}, err: String(e.message) };
      try {
        await m.cmd('WebDriver:Navigate', { url: `http://127.0.0.1:${PORT}/portal-unified.html` });
        await new Promise(r => setTimeout(r, 700));
        await m.cmd('WebDriver:SetWindowRect', { width: CURW, height: 900 });
        await new Promise(r => setTimeout(r, 300));
        return unwrap(await withTimeout(exec(), 25000));
      } catch (e2) { return { val: {}, err: 'execute-timeout(retry): ' + String(e2.message) }; }
    }
  };
  const runMany = async (script, name) => {
    const parts = Array.isArray(script) ? script : [script];
    let val = {}, err = null;
    for (let i = 0; i < parts.length; i++) {
      if (process.env.H9DBG) console.log('PART', name, i, 'start');
      const r = await run(parts[i]);
      if (process.env.H9DBG) console.log('PART', name, i, 'done', JSON.stringify(r.val).slice(0,140));
      if (r.val && r.val.__err) { err = r.val.__err; console.log('PART-ERR', name, i, r.val.__err); }
      if (r.err) err = r.err;
      if (r.val && typeof r.val === 'object') val = Object.assign(val, r.val);
      if (i < parts.length - 1) await new Promise(res => setTimeout(res, 450));
    }
    return { val, err };
  };
  for (const [name, script, width] of CASES) {
    if (process.env.H9ONLY && name !== process.env.H9ONLY) continue;
    await m.cmd('WebDriver:Navigate', { url: `http://127.0.0.1:${PORT}/portal-unified.html` });
    await new Promise(r => setTimeout(r, 600));
    CURW = width || 1440;
    await m.cmd('WebDriver:SetWindowRect', { width: CURW, height: 900 });
    await new Promise(r => setTimeout(r, 300));
    if (NEG_MODE) {
      const neg = NEG[name];
      if (!neg) { console.log(`NEG-SKIP ${name}`); continue; }
      const negRes = await run(neg);
      const injectErr = negRes && negRes.err ? negRes.err : null;
      if (injectErr) console.log(`NEG-BAD ${name} inject-error: ${injectErr}`);
      const { val, err } = await runMany(script, name);
      const got = (val && typeof val === 'object') ? val : null;
      const red = !!err || !got || Object.keys(got).length === 0 || Object.values(got).some(v => v !== true);
      if (!red) fail++;
      results[name] = { negative: true, red, assertions: val, error: err };
      console.log(`${red ? 'NEG-OK' : 'NEG-BAD'} ${name} ${JSON.stringify(val)}${err ? ' err=' + err : ''}`);
      continue;
    }
    const { val, err } = await runMany(script, name);
    const shotR = await m.cmd('WebDriver:TakeScreenshot', {}); const shot = typeof shotR === 'string' ? shotR : (shotR.value || shotR.screenshot || shotR.data);
    const png = path.join(OUT, name + '.png');
    fs.writeFileSync(png, Buffer.from(shot, 'base64'));
    const size = fs.statSync(png).size;
    const pass = !err && !!val && typeof val === 'object' && Object.keys(val).length > 0 && Object.values(val).every(v => v === true) && size > 0;
    if (!pass) fail++;
    results[name] = { pass, size, assertions: val, error: err };
    console.log(`${pass ? 'PASS' : 'FAIL'} ${name} size=${size} ${JSON.stringify(val)}${err ? ' err=' + err : ''}`);
  }
  fs.writeFileSync(path.join(OUT, NEG_MODE ? 'results-negative.json' : 'results.json'), JSON.stringify({ when: new Date().toISOString(), mode: NEG_MODE ? 'negative' : 'positive', fail, results }, null, 2));
  await m.cmd('WebDriver:DeleteSession', {}).catch(() => {});
  sock.end(); ff.kill(); srv.close();
  console.log(NEG_MODE ? (fail === 0 ? 'NEGATIVE SELF-CHECK: ALL SABOTAGED CASES RED AS EXPECTED' : `NEGATIVE SELF-CHECK ${fail} BAD`) : (fail === 0 ? 'HARNESS ALL GREEN' : `HARNESS ${fail} FAIL`));
  process.exit(fail === 0 ? 0 : 1);
}
main().catch(e => { console.error('HARNESS ERROR', e); process.exit(1); });