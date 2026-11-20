"""Real GTK window behind Start; its changing pixels must enter the glass, not wallpaper alone."""
from pathlib import Path
import importlib.util,json,sys
spec=importlib.util.spec_from_file_location('render_helpers',Path(__file__).with_name('start-rendering.py'))
helper=importlib.util.module_from_spec(spec);spec.loader.exec_module(helper)
B=helper.BRIDGE
PROGRAM=r'''
import gi,sys
from pathlib import Path
gi.require_version('Gtk','4.0');from gi.repository import Gtk,Gdk,GLib
control=Path(sys.argv[1]);app=Gtk.Application(application_id='org.winpoint.GlassProbe')
provider=Gtk.CssProvider();last=None
def update():
 global last
 color=control.read_text().strip()
 if color!=last:
  provider.load_from_string('window {background:'+color+';}');last=color
 return True
def activate(app):
 win=Gtk.ApplicationWindow(application=app,title='Private glass content probe');win.set_decorated(False)
 win.set_default_size(1150,840);Gtk.StyleContext.add_provider_for_display(Gdk.Display.get_default(),provider,800)
 update();GLib.timeout_add(80,update);win.present();win.maximize()
app.connect('activate',activate);app.run([])
'''
def mean(h,shot,rect):
 p=h.s.GdkPixbuf.Pixbuf.new_from_file(str(h.hm.RUN/'shots'/shot));data=p.get_pixels();nc=p.get_n_channels();rs=p.get_rowstride()
 x0,y0,w,ht=rect;return [round(sum(data[y*rs+x*nc+c] for y in range(y0,y0+ht) for x in range(x0,x0+w))/(w*ht),2) for c in range(3)]
def run(h):
 control=h.hm.RUN/'glass-color.txt';control.write_text('#f00010')
 probe=h.spawn('glass-source',['/usr/bin/python3','-c',PROGRAM,str(control)])
 h.until(lambda:h.s.window_of_pid(probe.pid),'Real GTK source window mapped',20)
 h.s.focus(probe.pid,'glass source');h.sleep(1)
 h.click('panel','tb-start');h.sleep(1)
 frame=h.s.window_of('io.github.win11shell.Start',h.native_pid)
 h.require('Start is open over the source window',bool(frame))
 region=(frame['x']+220,frame['y']+515,160,100)
 red=h.screenshot('start-over-red-window');r=mean(h,red,region)
 control.write_text('#10f010');h.sleep(0.8)
 green=h.screenshot('start-over-green-window');g=mean(h,green,region)
 h.check('Real window content changes the glass behind Start',r[0]-g[0]>10 and g[1]-r[1]>10,redMean=r,greenMean=g,region=region)
 status=h.eval(f'{B}.status()');h.record('windowScene',status)
 start=next(e for e in status['entries'] if e['kind']=='start')
 h.check('Start composes a window as well as the desktop',start.get('layers',0)>=2 and start.get('mode')==0,entry=start)
 # Start itself is never a source for its own glass, nor are the sibling glass widgets.
 own=h.eval(f"(()=>{{const f={B};const w=global.get_window_actors().find(a=>a.meta_window.get_wm_class()==='io.github.win11shell.Start');return [...f._entries.get(w.meta_window).layers.keys()].map(a=>a.meta_window?.get_wm_class()??'non-window');}})()")
 h.check('Only underlying window actors are sampled','io.github.win11shell.Start' not in own and 'non-window' not in own,sources=own)
 # A second real application tests stacking, not merely a color stored in a shared desktop texture.
 h.eval(helper.ESCAPE);h.sleep(0.35)
 second_control=h.hm.RUN/'glass-color-second.txt';second_control.write_text('#1010f0')
 second=h.spawn('glass-source-second',['/usr/bin/python3','-c',PROGRAM.replace('org.winpoint.GlassProbe','org.winpoint.GlassSecond'),str(second_control)])
 h.until(lambda:h.s.window_of_pid(second.pid),'Second GTK source mapped',20)
 h.s.focus(second.pid,'second glass source');h.sleep(0.6)
 h.click('panel','tb-start');h.sleep(0.6)
 blue_shot=h.screenshot('start-over-top-blue-window');blue=mean(h,blue_shot,region)
 h.check('Topmost underlying window wins over lower windows',blue[2]-blue[1]>10,blueMean=blue)
 h.eval(helper.ESCAPE);h.sleep(0.35);h.s.focus(probe.pid,'raise first glass source');h.sleep(0.4)
 h.click('panel','tb-start');h.sleep(0.6)
 raised_shot=h.screenshot('start-after-raising-green-window');raised=mean(h,raised_shot,region)
 h.check('Restacking updates the glass source',raised[1]-raised[2]>10,greenMean=raised)
 h.eval(helper.ESCAPE);h.sleep(0.35)
 second.terminate();second.wait(timeout=5)
 # Other-workspace windows must not be retained as visible glass sources.
 h.eval(f"(()=>{{const a=global.get_window_actors().find(a=>a.meta_window.get_pid()==={probe.pid});const wm=global.workspace_manager;if(wm.n_workspaces<2)wm.append_new_workspace(false,global.get_current_time());a.meta_window.change_workspace(wm.get_workspace_by_index(1));return true;}})()")
 h.sleep(0.6);h.click('panel','tb-start');h.sleep(0.6)
 away_shot=h.screenshot('start-with-source-on-other-workspace');away=mean(h,away_shot,region)
 h.check('Other-workspace content is excluded',sum(abs(away[i]-g[i]) for i in range(3))>15,awayMean=away,greenMean=g)
 h.eval(helper.ESCAPE);h.sleep(0.35)
 h.eval(f"global.get_window_actors().find(a=>a.meta_window.get_pid()==={probe.pid}).meta_window.change_workspace(global.workspace_manager.get_workspace_by_index(0));true")
 h.sleep(0.5);h.s.focus(probe.pid,'restored glass source');h.click('panel','tb-start');h.sleep(0.6)
 # A compositor fade must synchronously remove its independent sibling glass.
 result=h.eval(f"(()=>{{const f={B};const a=global.get_window_actors().find(a=>a.meta_window.get_wm_class()==='io.github.win11shell.Start');a.opacity=0;return f.status().entries.some(e=>e.kind==='start');}})()")
 h.check('Start fade cannot leave an orphan glass rectangle',not result)
 h.eval("global.get_window_actors().find(a=>a.meta_window.get_wm_class()==='io.github.win11shell.Start').opacity=255;true")
 h.until(lambda:any(e['kind']=='start' for e in h.eval(f'{B}.status()')['entries']),'Stable Start regains its glass',5)
 samples=[]
 for cycle in range(4):
  h.eval(helper.ESCAPE);h.sleep(0.35)
  h.click('panel','tb-start')
  for j in range(2):
   before=any(e['kind']=='start' for e in h.eval(f'{B}.status()')['entries'])
   shot=h.screenshot(f'opening-{cycle}-{j}',settle=0)
   after=any(e['kind']=='start' for e in h.eval(f'{B}.status()')['entries'])
   if before and after:samples.append(mean(h,shot,region))
  h.sleep(0.12)
 h.check('Opening never exposes untinted source content',len(samples)>=2 and all(max(v)<130 for v in samples),samples=samples)
 h.eval(helper.ESCAPE);h.sleep(0.4)
 h.eval(f"global.get_window_actors().find(a=>a.meta_window.get_pid()==={probe.pid}).meta_window.minimize();true")
 h.sleep(0.8);h.click('panel','tb-start');h.sleep(0.7)
 desktop=h.screenshot('start-with-source-minimized');d=mean(h,desktop,region)
 h.check('Minimized content leaves the glass',sum(abs(d[i]-g[i]) for i in range(3))>15,desktopMean=d,greenMean=g)
 h.eval(helper.ESCAPE);h.sleep(0.4)
 probe.terminate();probe.wait(timeout=5);h.sleep(0.6)
 h.check('Closed source snapshots are released',h.eval(f'{B}.status()')['sources']==1,status=h.eval(f'{B}.status()'))
 flags=h.eval(helper.FLAGS);h.check('No global redraw or continuous paint workaround',not bool(flags&(2|64)),flags=flags)
