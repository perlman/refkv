"""The RAF convention's source metadata (spec/zarrify/raf.md), for
spec/generate_schemas.py."""

INTEGERS = {"type": "array", "items": {"type": "integer", "minimum": 0}}

SCHEMA = {"anyOf": [{
    "type": "object",
    "description": "The root's: the header and its directory, the metadata records, the CFA region's TIFF, and "
                   "the sensor data's encoding "
                   "(spec/zarrify/raf.md §5 <https://github.com/d-v-b/refkv/blob/main/spec/zarrify/raf.md#5-source-metadata>)",
    "properties": {
        "header": {
            "type": "object",
            "description": "Every header field, in the file's order: texts as text, the reserved bytes as base64, "
                           "the directory as 12 integers",
            "additionalProperties": {"anyOf": [{"$ref": "#/$defs/text"}, {"type": "string"}, INTEGERS]},
        },
        "directory": {
            "type": "object",
            "description": "The directory's words, by name",
            "additionalProperties": {"type": "integer", "minimum": 0},
        },
        "records": {
            "type": "array",
            "description": "Every metadata record, in the file's order: its tag, and its bytes as base64",
            "items": {
                "type": "object",
                "properties": {"tag": {"type": "integer", "minimum": 0, "maximum": 65535}, "value": {"type": "string"}},
                "required": ["tag", "value"],
                "additionalProperties": False,
            },
        },
        "cfa_tiff": {
            "type": "object",
            "description": "The CFA region's TIFF: its byte order, IFD 0 and the raw IFD (tag 0xF000)",
            "properties": {
                "byte_order": {"enum": ["little", "big"]},
                "ifds": {"type": "array", "items": {
                    "type": "object",
                    "properties": {
                        "offset": {"type": "integer", "minimum": 0},
                        "entries": {"type": "array", "items": {
                            "type": "object",
                            "properties": {
                                "tag": {"type": "integer"}, "type": {"type": "integer"}, "count": {"type": "integer"},
                                "value": {"anyOf": [{"type": "integer"}, {"type": "string"}]},
                            },
                            "required": ["tag", "type", "count", "value"],
                            "additionalProperties": False,
                        }},
                    },
                    "required": ["offset", "entries"],
                    "additionalProperties": False,
                }},
            },
            "required": ["byte_order", "ifds"],
            "additionalProperties": False,
        },
        "encoding": {"enum": ["compressed", "uncompressed"]},
        "byte_order": {"enum": ["little", "big"], "description": "Uncompressed samples' byte order"},
        "stripes": {
            "type": "object",
            "description": "A compressed strip's header, and each block's bytes",
            "properties": {
                **{k: {"type": "integer", "minimum": 0} for k in (
                    "lossless", "raw_type", "bits", "height", "rounded_width", "width", "block_size", "blocks",
                    "row_groups")},
                "block_sizes": INTEGERS,
            },
            "required": ["lossless", "raw_type", "bits", "height", "rounded_width", "width", "block_size", "blocks",
                         "row_groups", "block_sizes"],
            "additionalProperties": False,
        },
        "cfa": {
            "type": "object",
            "description": "The pattern the codec places compressed samples by, and whether the file states it",
            "properties": {
                "pattern": {"type": "array", "items": {"type": "array", "items": {"enum": [0, 1, 2]}}},
                "from": {"enum": ["file", "readers"]},
            },
            "required": ["pattern", "from"],
            "additionalProperties": False,
        },
    },
    "required": ["header", "directory", "records", "cfa_tiff", "encoding"],
    "additionalProperties": False,
},
    {"$ref": "#/$defs/mirror"},
    {"$ref": "#/$defs/mirrorView"},
]}
