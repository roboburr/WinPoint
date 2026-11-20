"""Start avatar and desktop-glass regression (private GNOME only).
Run with --lang nl --profile-picture tests/gnome/fixtures/avatar-wide.png.
The synthetic 1600x900 image must stay 32x32. Real Start and Settings dropdown
input must keep the panel actor/effect alive. A bright actor inserted behind the
taskbar must not enter its desktop-only blur. No global redraw flags are changed.
"""
from pathlib import Path
import importlib.util
spec=importlib.util.spec_from_file_location('render_helpers',Path(__file__).with_name('start-rendering.py'))
helper=importlib.util.module_from_spec(spec);spec.loader.exec_module(helper)
B=helper.BRIDGE
INSTALL="""(()=>{const f=BRIDGE;globalThis.__bgProbe={changes:[],last:'',frames:0};
const panel=global.get_window_actors().find(a=>a.meta_window.get_wm_class()==='io.github.win11shell.Panel');
globalThis.__probePanel=panel;globalThis.__probePanelEntry=f._entries.get(panel.meta_window);
globalThis.__probePanelEffect=globalThis.__probePanelEntry?.effect;
globalThis.__probePaintSignal=global.stage.connect('after-paint',()=>{
 const p=globalThis.__bgProbe,e=f._entries.get(panel.meta_window);
 const w=global.display.focus_window;
 const app=[...f._entries.values()].filter(e=>e.kind==='window').map(e=>({effect:!!e.effect,focused:e.focused}));
 const state={panelMapped:panel.mapped,panelVisible:panel.visible,panelOpacity:panel.opacity,
 panelBlur:!!e?.effect,samePanelEntry:e===globalThis.__probePanelEntry,samePanelEffect:e?.effect===globalThis.__probePanelEffect,
 focus:w?.get_wm_class()??'',app};
 const text=JSON.stringify(state);p.frames++;if(text!==p.last){p.changes.push(state);p.last=text;}
});return true;})()""".replace('BRIDGE',B)

def changed_pixels(h,a,b,region):
    aa,bb=[h.s.GdkPixbuf.Pixbuf.new_from_file(str(h.hm.RUN/'shots'/x)) for x in (a,b)]
    da,db=aa.get_pixels(),bb.get_pixels();x0,y0,w,ht=region;changed=0
    for y in range(y0,y0+ht):
        for x in range(x0,x0+w):
            ia=y*aa.get_rowstride()+x*aa.get_n_channels();ib=y*bb.get_rowstride()+x*bb.get_n_channels()
            changed+=max(abs(da[ia+c]-db[ib+c]) for c in range(3))>2
    return changed

def run(h):
    h.sleep(2)
    h.eval(INSTALL)
    flags=h.eval("(await import('gi://Clutter')).default.get_debug_flags()[1]")
    h.check('No global full-frame or continuous-redraw flags',not bool(flags & (2|64)),flags=flags)
    for i in range(3):
        h.click('panel','tb-start');h.sleep(0.5)
        ns=helper.nodes(h)
        avatars=[n for n in ns if 'w11-avatar' in n.get('classes',[])]
        h.require('Profile photo is loaded',bool(avatars) and any(n['type']=='GtkPicture' for a in avatars for n in helper.walk(a)))
        h.check('Start photo stays 32 by 32 '+str(i),all(n['rect'][2:]==[32,32] for n in avatars),avatars=[n['rect'] for n in avatars])
        if i==0:h.screenshot('profile-photo')
        h.eval(helper.ESCAPE);h.sleep(0.5)
    h.open('settings:colors')
    h.until(lambda:h.find('settings',name='colors-mode'), 'Settings dropdown available',20)
    h.sleep(1)
    h.screenshot('settings-before-dropdown')
    for i in range(4):
        h.click('settings','colors-mode');h.sleep(0.4)
        if i==0:h.screenshot('settings-dropdown-open')
        h.eval(helper.ESCAPE);h.sleep(0.4)
    h.screenshot('settings-dropdown-closed')
    entries=h.eval(f'{B}.status()')['entries']
    h.check('Blur composes isolated underlying window actors',all(e.get('source')=='windows' and e.get('mode')==0 for e in entries if e['effect']),entries=entries)
    entry=next(e for e in entries if e['kind']=='taskbar')
    r=entry['rect'];band=(r['x']+8,r['y']+3,r['w']-240,r['h']-6)
    before=h.screenshot('taskbar-before-background-probe')
    h.eval("""(async()=>{const St=(await import('gi://St')).default;const f=BRIDGE;
    const e=[...f._entries.values()].find(e=>e.kind==='taskbar');
    globalThis.__brightBg=new St.Widget({reactive:false,style:'background-color:rgb(255,0,80);'});
    globalThis.__brightBg.set_position(e.rect.x,e.rect.y);globalThis.__brightBg.set_size(e.rect.w,e.rect.h);
    e.outer.get_parent().insert_child_below(globalThis.__brightBg,e.outer);return true;})()""".replace('BRIDGE',B))
    after=h.screenshot('taskbar-with-background-probe')
    changed=changed_pixels(h,before,after,band)
    h.check('Foreground changes behind taskbar cannot contaminate its glass',changed<=30,changedPixels=changed)
    h.eval('globalThis.__brightBg.destroy()')

    trace=h.eval('(()=>{global.stage.disconnect(globalThis.__probePaintSignal);return globalThis.__bgProbe;})()')
    h.record('backgroundTrace',trace)
    h.check('Taskbar actor never hides or fades',all(x['panelMapped'] and x['panelVisible'] and x['panelOpacity']==255 for x in trace['changes']),trace=trace)
    h.check('Taskbar blur is retained without recreation',all(x['panelBlur'] and x['samePanelEntry'] and x['samePanelEffect'] for x in trace['changes']),trace=trace)

    h.sleep(0.5)
    captures=h.eval(f'{B}.status()')['sourceCaptures']
    h.sleep(1)
    h.check('Idle desktop source is not recaptured',h.eval(f'{B}.status()')['sourceCaptures']==captures,captures=captures)
    import json
    options=h.eval(f'({{enabled:true,...{B}._options}})')
    h.eval(f'{B}.configure({{enabled:false}})')
    h.check('Disabling glass releases every cached source',h.eval(f'{B}.status()')['sources']==0)
    h.eval(f'{B}.configure({json.dumps(options)})')
    h.check('Re-enabled glass restores only used sources',1 <= h.eval(f'{B}.status()')['sources'] <= 3)
