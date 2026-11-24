#!/bin/zsh
set -eu
# All input targets a uniquely identified app with synthetic notes in a scratch directory.
# Usage: packaging/profile-editor.sh target/Polar.app [paragraphs] [plain|math|math_unique|tasks|bullets|numbers|images|blocks] [query]
script_dir=${0:A:h}
scratch=$(mktemp -d /tmp/polar-profile.XXXXXX)
swiftc "$script_dir/performance/type.swift" -o "$scratch/type"
swiftc "$script_dir/performance/window.swift" -o "$scratch/window"
app="$scratch/Typing Test.app"
ditto "$1" "$app"
plutil -replace CFBundleIdentifier -string "com.alessioforgione.polar.profile.$(date +%s)" "$app/Contents/Info.plist"
codesign --force --sign - "$app"
mkdir "$scratch/data"
python3 - "$scratch/data/notes.json" "${2:-200}" "${3:-plain}" <<'PY'
import json,sys
count=int(sys.argv[2]); mode=sys.argv[3]
paragraph={
 'plain': 'Paragrafo con **grassetto**, *corsivo* e una frase di prova.\n\n',
 'math': 'Paragrafo con **grassetto** e $E=mc^2$.\n\n',
 'math_unique': '',
 'tasks': '- [ ] Un compito con **grassetto** e testo.\n',
 'bullets': '- Una voce con **grassetto** e testo.\n',
 'numbers': '1. Una voce con **grassetto** e testo.\n',
 'blocks': 'Paragrafo.\n\n| Colonna | Valore |\n| --- | --- |\n| Riga | Testo |\n\n> [!TIP] Suggerimento\n> Testo del callout.\n\n---\n\n',
 'images': 'Testo con **grassetto**.\n\n![Immagine](attachments/image-test.png)\n\n',
}[mode]
notes=[dict(id=i+1,text=f'# Nota {i+1}\n\n'+(paragraph*(count if i==0 else 20))+'\n#viaggi/idee',pinned=False,trashed=False,modified=1794230000-i) for i in range(100)]
if mode == 'math_unique':
 for i,note in enumerate(notes):
  note['text'] = f'# Nota {i+1}\n\n'+''.join(f'Paragrafo **formattato** $x_{{{j}}} + 1$.\n\n' for j in range(count if i==0 else 20))+'\n#viaggi/idee'
json.dump(dict(version=2,notes=notes),open(sys.argv[1],'w'))
open(sys.argv[1]+'.original','w').write(notes[0]['text'])
PY
if [ "${3:-plain}" = images ]; then
  mkdir "$scratch/data/attachments"
  magick -size 1600x1000 gradient:white-gray "$scratch/data/attachments/image-test.png"
fi
POLAR_DATA_DIR="$scratch/data" POLAR_PROFILE=1 "$app/Contents/MacOS/polar" -ApplePersistenceIgnoreState YES >"$scratch/run.log" 2>&1 &
pid=$!
trap 'kill "$pid" 2>/dev/null || true; wait "$pid" 2>/dev/null || true' EXIT
sleep 4
ui="$scratch/window"
read -r id left top width height <<< "$("$ui" window "$pid" | head -1)"
if [ -n "${4:-}" ]; then
  "$scratch/type" "$pid" --find
  "$scratch/type" "$pid" "$4"
  sleep 1
fi
"$ui" click "$pid" $((left+700)) $((top+150))
"$scratch/type" "$pid" 'Verifica rapida della digitazione. '
sleep 1
python3 - "$scratch/data/notes.json" <<'PY'
import json,sys
assert 'Verifica rapida della digitazione. ' in json.load(open(sys.argv[1]))['notes'][0]['text'], 'Editor did not receive the typed text'
PY
"$scratch/type" "$pid" --delete 35
sleep 3
python3 - "$scratch/run.log" <<'PY'
import sys,collections,statistics
values=collections.defaultdict(list)
for line in open(sys.argv[1]):
 if line.startswith('POLAR_PROFILE '):
  _,key,value=line.split();values[key].append(float(value))
assert len(values['key']) >= 70, 'The editor did not process all insertions and deletions'
for key,vs in values.items():
 print(key,'n=',len(vs),'median=',round(statistics.median(vs),2),'p95=',sorted(vs)[int((len(vs)-1)*.95)],'max=',max(vs))
PY
python3 - "$scratch/data/notes.json" <<'PY'
import json,sys
note=json.load(open(sys.argv[1]))['notes'][0]
assert note['text'] == open(sys.argv[1]+'.original').read(), 'Typing/deletion changed the original note'
print('Typing and deletion completed')
PY
echo "$scratch"
