#!/usr/bin/env sh
# Rebuilds the fonts embedded in Debrief's PDF reports (SIL OFL 1.1, see OFL-*.txt).
#
# Sources (the same families the UI ships via @fontsource):
#   IBM Plex Sans 3.201 (variable)  https://github.com/google/fonts/tree/main/ofl/ibmplexsans
#   JetBrains Mono 2.304            https://github.com/JetBrains/JetBrainsMono/tree/master/fonts/ttf
#
# Needs Python 3 with fontTools (`pip install fonttools==4.55.0`). Usage:
#   build-fonts.sh <IBMPlexSans[wdth,wght].ttf> <IBMPlexSans-Italic[wdth,wght].ttf> <JetBrainsMono-Regular.ttf>
#
# Static instances are cut from the variable Plex fonts, then every font is reduced to
# Latin (incl. Latin-1, Extended-A/B and Additional), Greek, Cyrillic, punctuation,
# currency, letterlike symbols, arrows, maths operators, box drawing, geometric shapes
# and U+FFFD, without hinting or OpenType layout tables (the PDF writer does no shaping).
#
# "Plex" is a Reserved Font Name under IBM's OFL, so the reduced (modified) Plex fonts are
# renamed "Debrief Report Sans" as OFL condition 3 requires; JetBrains Mono has no reserved
# name and is renamed "Debrief Report Mono" only for consistency.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
unicodes="U+0020-007E,U+00A0-024F,U+0300-036F,U+0370-03FF,U+0400-052F,U+1E00-1EFF,U+2000-206F,U+20A0-20CF,U+2100-214F,U+2190-21FF,U+2200-22FF,U+2500-257F,U+25A0-25FF,U+2713,U+2717,U+FFFD"
python3 -m fontTools.varLib.instancer -q -o "$tmp/regular.ttf" "$1" wght=400 wdth=100
python3 -m fontTools.varLib.instancer -q -o "$tmp/bold.ttf" "$1" wght=700 wdth=100
python3 -m fontTools.varLib.instancer -q -o "$tmp/italic.ttf" "$2" wght=400 wdth=100
cp "$3" "$tmp/mono.ttf"
for spec in "regular DebriefReportSans-Regular Debrief_Report_Sans Regular" \
            "bold DebriefReportSans-Bold Debrief_Report_Sans Bold" \
            "italic DebriefReportSans-Italic Debrief_Report_Sans Italic" \
            "mono DebriefReportMono-Regular Debrief_Report_Mono Regular"; do
  set -- $spec
  python3 -m fontTools.subset "$tmp/$1.ttf" --unicodes="$unicodes" --layout-features='' --no-hinting \
    --name-IDs='0,1,2,3,4,5,6,13,14' --drop-tables+=GSUB,GPOS,GDEF,DSIG,STAT,MVAR,HVAR \
    --output-file="$tmp/$2.ttf"
  python3 -I - "$tmp/$2.ttf" "$here/$2.ttf" "$(echo "$3" | tr _ ' ')" "$4" "$2" <<'PY'
import sys
from fontTools.ttLib import TTFont
src, dst, family, style, ps = sys.argv[1:]
font = TTFont(src)
names = font["name"]
for rec in list(names.names):
    if rec.nameID in (1, 2, 3, 4, 6, 16, 17):
        names.removeNames(nameID=rec.nameID)
for nid, value in ((1, family), (2, style), (3, ps), (4, f"{family} {style}"), (6, ps)):
    names.setName(value, nid, 3, 1, 0x409)
font.save(dst)
PY
done
