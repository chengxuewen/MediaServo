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
    return {open,hiddenClosed,initialFocus,tabs,roving,trap,modalFirst,esc,restored,list,closedInert};`],
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
  floor: `GROUPS.find(g=>g.id==='talk-bravo').q=[];`,
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
  'accounts-narrow': `var st=document.createElement('style');st.textContent='@media (max-width:760px){#topbar{gap:var(--sp12);padding:0 var(--sp16)}}';document.head.appendChild(st);`,
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
  await new Promise(r => setTimeout(r, 4000));
  const sock = await new Promise((res, rej) => { const s = net.connect(2828, '127.0.0.1', () => res(s)); s.on('error', rej); });
  await new Promise(r => sock.once('data', r)); // handshake
  const m = new Marionette(sock);
  await m.cmd('WebDriver:NewSession', {});
  const results = {}; let fail = 0;
  const NEG_MODE = process.argv.includes('--negative');
  const run = async (src) => {
    const inner = 'try{window.__result=(function(){' + src + '})();}catch(e){window.__result={__err:String(e && e.message || e)+" @"+String(e && e.stack || "").slice(0,220)};}';
    const wrapped = "var s=document.createElement('script');s.textContent=" + JSON.stringify(inner) + ";document.documentElement.appendChild(s);var r=window.__result;window.__result=undefined;return r;";
    try { const r = await m.cmd('WebDriver:ExecuteScript', { script: wrapped, args: [] }); return { val: (r && typeof r === 'object' && 'value' in r) ? r.value : r, err: null }; }
    catch (e) { return { val: {}, err: String(e.message) }; }
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
    await m.cmd('WebDriver:SetWindowRect', { width: width || 1440, height: 900 });
    await new Promise(r => setTimeout(r, 300));
    if (NEG_MODE) {
      const neg = NEG[name];
      if (!neg) { console.log(`NEG-SKIP ${name}`); continue; }
      await run(neg);
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