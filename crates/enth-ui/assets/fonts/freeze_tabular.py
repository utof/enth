"""Derive InterTabular-Regular.ttf from Inter-Regular.ttf (Inter 4.1, OFL-1.1).

egui 0.37 shapes text with harfrust but applies no OpenType features, so Inter's `tnum`
(tabular figures) cannot be switched on at run time. This script freezes `tnum` into the cmap
for the characters that appear in parameter values and drops everything else. egui falls back
to Inter Regular for any other glyph, so the subset only needs these characters.

Usage: python3 -I freeze_tabular.py Inter-Regular.ttf InterTabular-Regular.ttf
Needs fontTools (tested with 4.46.0).
"""
import sys

from fontTools import subset
from fontTools.ttLib import TTFont

# Digits and the punctuation that sits between them in values ("-6.0 dB", "1:4", "+35 %").
CHARS = "0123456789.,:+-− "
# Figure space: a blank as wide as a digit, for padding values to a fixed width.
EXTRA = " "


def tnum_map(font):
    gsub = font["GSUB"].table
    lookups = set()
    for record in gsub.FeatureList.FeatureRecord:
        if record.FeatureTag == "tnum":
            lookups.update(record.Feature.LookupListIndex)
    mapping = {}
    for index in sorted(lookups):
        lookup = gsub.LookupList.Lookup[index]
        for table in lookup.SubTable:
            if lookup.LookupType == 7:  # extension
                table = table.ExtSubTable
            mapping.update(getattr(table, "mapping", {}))
    return mapping


def main(source, target):
    font = TTFont(source, recalcTimestamp=False)
    tnum = tnum_map(font)
    best = font.getBestCmap()
    frozen = {}
    for char in CHARS:
        glyph = best[ord(char)]
        frozen[ord(char)] = tnum.get(glyph, glyph)
    for table in font["cmap"].tables:
        if table.isUnicode():
            for codepoint, glyph in frozen.items():
                if codepoint in table.cmap:
                    table.cmap[codepoint] = glyph

    options = subset.Options()
    options.layout_features = []  # nothing left to substitute
    options.name_IDs = ["*"]
    options.notdef_outline = True
    subsetter = subset.Subsetter(options)
    subsetter.populate(unicodes=[ord(c) for c in CHARS + EXTRA])
    subsetter.subset(font)

    names = font["name"]
    for record in list(names.names):
        if record.nameID in (1, 16):
            record.string = "Inter Tabular"
        elif record.nameID == 4:
            record.string = "Inter Tabular Regular"
        elif record.nameID == 6:
            record.string = "InterTabular-Regular"
        elif record.nameID == 3:
            record.string = "InterTabular-Regular;derived from Inter 4.1"
        elif record.nameID == 5:
            record.string = str(record) + "; tnum frozen into cmap, subset to value characters (enth)"
    font.save(target)


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
