"""Offline filtering contracts; Android source/MMKV I/O requires device proof."""
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


def _java_bin() -> Path:
    for candidate in (
        os.environ.get('JAVA_HOME'),
        str(Path.home() / '.local/opt/android-studio/jbr'),
    ):
        if candidate and (Path(candidate) / 'bin/javac').is_file():
            return Path(candidate) / 'bin'
    javac = shutil.which('javac')
    if javac:
        return Path(javac).resolve().parent
    return Path.home() / '.local/opt/android-studio/jbr/bin'


def _android_jar() -> Path | None:
    for sdk in (
        os.environ.get('ANDROID_SDK_ROOT'),
        os.environ.get('ANDROID_HOME'),
        str(Path.home() / 'Android/Sdk'),
    ):
        if not sdk:
            continue
        preferred = Path(sdk) / 'platforms/android-36/android.jar'
        if preferred.is_file():
            return preferred
        jars = sorted((Path(sdk) / 'platforms').glob('android-*/android.jar'), reverse=True)
        if jars:
            return jars[0]
    return None


class ModelPreferencesTest(unittest.TestCase):
    def test_only_valid_registered_model_selection_is_returned(self):
        java_bin = _java_bin()
        android_jar = _android_jar()
        with tempfile.TemporaryDirectory(prefix='stepx-model-preferences-') as tmp:
            work = Path(tmp)
            json_source = work / 'org/json/JSONObject.java'
            json_source.parent.mkdir(parents=True)
            json_source.write_text('''package org.json;
import java.util.*;import java.util.regex.*; public class JSONObject {
private Map<String,Object> values=new LinkedHashMap<>();
public JSONObject(){} public JSONObject(String value){
if(value!=null){Matcher m=Pattern.compile("\\"([^\\"]+)\\"\\\\s*:\\\\s*(\\"([^\\"]*)\\"|(-?\\\\d+))").matcher(value);
while(m.find()){values.put(m.group(1),m.group(3)!=null?m.group(3):Integer.valueOf(m.group(4)));}}}
public JSONObject put(String key,Object value){values.put(key,value);return this;}
public Object opt(String key){return values.get(key);}
public String toString(){return values.toString();}
}''')
            harness = work / 'ModelPreferencesContract.java'
            harness.write_text('''import java.lang.reflect.*;import java.util.*;
public class ModelPreferencesContract {
static void check(boolean value){if(!value)throw new AssertionError();}
static String selection(String id,String map)throws Exception{
Class<?> type=Class.forName("stepx.NativeGuiModelPreferences");
Method describe=type.getDeclaredMethod("describeSelection",String.class,String.class,String.class,Map.class,org.json.JSONObject.class);
describe.setAccessible(true);org.json.JSONObject out=new org.json.JSONObject();
describe.invoke(null,id,map,"fixed-default",Map.of("fixed-default","StepGelabModelCell"),out);return out.toString();}
public static void main(String[]args)throws Exception{
check(selection(null,null).contains("no_saved_selection"));
check(selection("",null).contains("no_saved_selection"));
String standard=selection("fixed-default",null);
check(standard.contains("current_model_id=fixed-default")&&standard.contains("matches_static_default=true"));
check(selection("https://private.invalid/account?token=SECRET",null).contains("invalid_selection"));
check(!selection("https://private.invalid/account?token=SECRET",null).contains("SECRET"));
check(selection("../SECRET",null).contains("invalid_selection"));
check(selection("x".repeat(161),null).contains("invalid_selection"));
check(selection("opaque-secret",null).contains("unregistered_selection"));
check(!selection("opaque-secret",null).contains("opaque-secret"));
String custom=selection("provider/valid-gui","{\\"provider/valid-gui\\":\\"GeminiModelCell\\",\\"token\\":\\"SECRET_CREDENTIAL\\"}");
check(custom.contains("current_model_id=provider/valid-gui")&&custom.contains("model_cell_type=GeminiModelCell"));
check(custom.contains("custom_model=true")&&!custom.contains("SECRET")&&!custom.contains("token"));
String rejected=selection("opaque-secret","{\\"opaque-secret\\":\\"SECRET_CUSTOM_CLASS\\"}");
check(rejected.contains("unregistered_selection")&&!rejected.contains("SECRET")&&!rejected.contains("opaque-secret"));
check(selection("opaque-secret","{\\"opaque-secret\\":42}").contains("unregistered_selection"));
try{selection("test","x".repeat(131073));throw new AssertionError();}
catch(InvocationTargetException expected){check(expected.getCause() instanceof SecurityException);}
System.out.println("model selection filtering contracts passed");
}}
''')
            cp = str(android_jar) if android_jar else ''
            javac_cmd = [str(java_bin / 'javac'), '--release', '11']
            if cp:
                javac_cmd.extend(['-cp', cp])
            javac_cmd.extend([
                '-d', str(work / 'classes'),
                str(json_source),
                str(harness),
                str(ROOT / 'desktop/gui-native/NativeGuiModelPreferences.java'),
            ])
            subprocess.run(javac_cmd, check=True, capture_output=True, text=True)
            run_cp = f'{work / "classes"}:{cp}' if cp else str(work / 'classes')
            result = subprocess.run([str(java_bin / 'java'), '-cp', run_cp,
                                     'ModelPreferencesContract'], check=True, capture_output=True, text=True)
            self.assertIn('contracts passed', result.stdout)


if __name__ == '__main__':
    unittest.main()
