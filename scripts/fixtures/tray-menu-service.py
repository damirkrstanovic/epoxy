"""Synthetic menu only: never imports NSS, opens a card or performs signing."""
import dbus, dbus.service, dbus.mainloop.glib
from gi.repository import GLib
import os

dbus.mainloop.glib.DBusGMainLoop(set_as_default=True)
bus = dbus.SessionBus()
name = dbus.service.BusName(f'org.kde.StatusNotifierItem-{os.getpid()}-1', bus)

def props(label, submenu=False, enabled=True):
    p={'label':dbus.String(label)}
    if not enabled: p['enabled']=dbus.Boolean(False)
    if submenu: p['children-display']=dbus.String('submenu')
    return p
nodes={0:(props('Root',True),[1,8]),1:(props('Certificates',True),[2,7]),2:(props('Selected certificate',True),[3,4]),3:(props('Used for signing',enabled=False),[]),4:(props('Certificate details',True),[5,6]),5:(props('Issued by: Synthetic CA',enabled=False),[]),6:(props('Valid until: 2090',enabled=False),[]),7:(props('Refresh certificates'),[]),8:(props('Quit fixture'),[])}
def layout(i,depth,fields,variant=0):
    p,kids=nodes[i]
    if fields: p={k:v for k,v in p.items() if k in fields}
    children=[] if depth==0 else [layout(k,depth-1,fields,1) for k in kids]
    return dbus.Struct((dbus.Int32(i),dbus.Dictionary(p,signature='sv'),dbus.Array(children,signature='v')),signature='ia{sv}av',variant_level=variant)

class Menu(dbus.service.Object):
    @dbus.service.method('com.canonical.dbusmenu',in_signature='iias',out_signature='u(ia{sv}av)')
    def GetLayout(self,parent,depth,fields):
        print('GetLayout',int(parent),int(depth),flush=True)
        return dbus.UInt32(1),layout(parent,depth,fields)
    @dbus.service.method('com.canonical.dbusmenu',in_signature='i',out_signature='b')
    def AboutToShow(self,i): return False
    @dbus.service.method('com.canonical.dbusmenu',in_signature='isvu',out_signature='')
    def Event(self,i,event,data,timestamp): print('Event',int(i),str(event),flush=True)
    @dbus.service.method('org.freedesktop.DBus.Properties',in_signature='s',out_signature='a{sv}')
    def GetAll(self,interface): return {'Version':dbus.UInt32(3),'TextDirection':'ltr','Status':'normal','IconThemePath':dbus.Array([],signature='s')}
class Item(dbus.service.Object):
    @dbus.service.method('org.freedesktop.DBus.Properties',in_signature='s',out_signature='a{sv}')
    def GetAll(self,interface): return {'Id':'dev.semyon.epoxy','Title':'Epoxy synthetic','Category':'ApplicationStatus','Status':'Active','WindowId':dbus.UInt32(0),'ItemIsMenu':True,'Menu':dbus.ObjectPath('/MenuBar'),'IconName':'dialog-information','IconPixmap':dbus.Array([],signature='(iiay)'),'AttentionIconName':'','AttentionIconPixmap':dbus.Array([],signature='(iiay)'),'OverlayIconName':'','OverlayIconPixmap':dbus.Array([],signature='(iiay)'),'IconThemePath':'','ToolTip':dbus.Struct(('',dbus.Array([],signature='(iiay)'),'Synthetic',''),signature='sa(iiay)ss')}
    @dbus.service.method('org.freedesktop.DBus.Properties',in_signature='ss',out_signature='v')
    def Get(self,interface,property): return self.GetAll(interface)[property]
menu=Menu(bus,'/MenuBar'); item=Item(bus,'/StatusNotifierItem')
def register():
    try:
        watcher=dbus.Interface(bus.get_object('org.kde.StatusNotifierWatcher','/StatusNotifierWatcher'),'org.kde.StatusNotifierWatcher')
        watcher.RegisterStatusNotifierItem(name.get_name()); print('registered',flush=True); return False
    except dbus.DBusException: return True
GLib.timeout_add(100,register)
GLib.MainLoop().run()
