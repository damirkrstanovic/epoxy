import importlib.util
from pathlib import Path
import unittest
spec = importlib.util.spec_from_file_location('inspect_tray', Path(__file__).with_name('inspect-tray.py'))
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)

class InspectorTests(unittest.TestCase):
    def test_nested_layout_preserves_navigation_and_redacts_identity(self):
        layout = (0, {}, [(1, {'label':'Certificates','children-display':'submenu'}, [
            (2, {'label':'✓ Synthetic secret identity','children-display':'submenu'}, [
                (3, {'label':'Used for signing','enabled':False}, []),
                (4, {'label':'Issued by: Secret CA'}, [])])])])
        rows = m.describe_layout(layout)
        self.assertEqual([r['id'] for r in rows], [0,1,2,3,4])
        self.assertEqual(rows[2]['label'], '[selected certificate]')
        self.assertTrue(rows[2]['enabled'])
        self.assertTrue(rows[2]['submenu'])
        self.assertEqual(rows[2]['children'], 2)
        self.assertFalse(rows[3]['enabled'])
        self.assertNotIn('secret', str(rows).lower())

    def test_event_summary_never_emits_payloads_or_unknown_strings(self):
        self.assertEqual(m.event_summary('Event', [42,'clicked',{'pin':'synthetic-secret'},123]),
                         {'method':'Event','id':42,'event':'clicked'})
        self.assertNotIn('synthetic-secret', str(m.event_summary('Event',[42,'synthetic-secret','synthetic-secret',0])))
        self.assertIsNone(m.event_summary('Unknown',['synthetic-secret']))
        self.assertEqual(m.event_summary('GetLayout',[12,-1,['label']]), {'method':'GetLayout','id':12,'depth':-1})

if __name__ == '__main__': unittest.main()
