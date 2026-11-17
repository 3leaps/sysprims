"""Load GitHub Actions workflow files for the workflow lints (standard library only).

A full YAML parse, so permissions, steps and `uses` are read the way Actions
reads them (quoted keys and scalars, flow style, any indentation). The parse
is done by the yamlprims CLI through scripts/yaml_json.py (YAML 1.2 Core
Schema, so `on` is the string "on"; the binary is authenticated by its
committed digest, config/yamlprims-cli.txt). Constructs that make a value
depend on something other than its own text are refused: aliases, merge keys,
explicit tags and duplicate keys. Workflows here use none of them; refusing
them keeps the lints' view equal to the runner's.

A failure of the reader itself (a missing or unpinned binary, a crash, a
timeout) raises yaml_json.ReaderError, never WorkflowError, so it can never
be counted as a lint finding.

`compose` returns the node tree with source positions (`start_mark` and
`end_mark`, 0-based `line` and `column`, columns in characters) for lints
that read or rewrite workflow text by position.
"""

import importlib.util
import os
import pathlib
import re
import struct


def _load_reader():
    path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "yaml_json.py")
    spec = importlib.util.spec_from_file_location("sysprims_yaml_json", path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


yaml_json = _load_reader()
ReaderError = yaml_json.ReaderError

STR = "tag:yaml.org,2002:str"
INT = "tag:yaml.org,2002:int"
FLOAT = "tag:yaml.org,2002:float"
BOOL = "tag:yaml.org,2002:bool"
NULL = "tag:yaml.org,2002:null"
SEQ = "tag:yaml.org,2002:seq"
MAP = "tag:yaml.org,2002:map"
_SCALAR_TAGS = {"string": STR, "int": INT, "float": FLOAT, "bool": BOOL, "null": NULL}


class WorkflowError(Exception):
    pass


class Mark:
    __slots__ = ("line", "column")

    def __init__(self, line, column):
        self.line = line
        self.column = column


class Node:
    style = None

    def __init__(self, tag, value, span):
        self.tag = tag
        self.value = value
        self.start_mark = Mark(span[1] - 1, span[2] - 1)
        self.end_mark = Mark(span[4] - 1, span[5] - 1)


class ScalarNode(Node):
    pass


class SequenceNode(Node):
    pass


class MappingNode(Node):
    pass


def _span(node):
    span = node.get("span")
    if not (
        isinstance(span, list)
        and len(span) == 6
        and all(isinstance(n, int) and n >= 0 for n in span)
    ):
        raise ReaderError("yamlprims output has a node without a source span")
    return span


def _float(bits):
    if not (isinstance(bits, str) and re.fullmatch(r"[0-9a-f]{16}", bits)):
        raise ReaderError("yamlprims output has a malformed float")
    return struct.unpack(">d", bytes.fromhex(bits))[0]


_STYLES = {b"'": "'", b'"': '"', b"|": "|", b">": ">"}


def _node(node, source):
    """One yamlprims-value/1 node as a Node; refuses aliases, merges and tags.
    A scalar's style (None for plain, or the quote or block indicator) is the
    source byte at the start of its span."""
    if not isinstance(node, dict):
        raise ReaderError("yamlprims output has a malformed node")
    if node.get("via") is not None:
        raise WorkflowError(
            "anchors, aliases and merge keys are not accepted in workflows"
        )
    if node.get("tag_start") is not None or node.get("kind") == "tagged":
        raise WorkflowError("explicit tags are not accepted in workflows")
    kind = node.get("kind")
    span = _span(node)
    if kind == "mapping":
        entries = node.get("entries")
        if not isinstance(entries, list):
            raise ReaderError("yamlprims output has a malformed mapping")
        pairs = []
        for entry in entries:
            if not isinstance(entry, dict):
                raise ReaderError("yamlprims output has a malformed mapping entry")
            pairs.append(
                (_node(entry.get("key"), source), _node(entry.get("value"), source))
            )
        return MappingNode(MAP, pairs, span)
    if kind == "sequence":
        items = node.get("items")
        if not isinstance(items, list):
            raise ReaderError("yamlprims output has a malformed sequence")
        return SequenceNode(SEQ, [_node(item, source) for item in items], span)
    if kind not in _SCALAR_TAGS:
        raise ReaderError(f"yamlprims output has an unknown node kind {kind!r}")
    if kind == "null":
        text = "null"
    elif kind == "bool":
        if not isinstance(node.get("value"), bool):
            raise ReaderError("yamlprims output has a malformed bool")
        text = "true" if node["value"] else "false"
    elif kind == "float":
        text = repr(_float(node.get("bits")))
    else:
        text = node.get("value")
        if not isinstance(text, str):
            raise ReaderError(f"yamlprims output has a malformed {kind}")
    scalar = ScalarNode(_SCALAR_TAGS[kind], text, span)
    scalar.wire = node
    scalar.style = (
        _STYLES.get(source[span[0] : span[0] + 1]) if span[0] < span[3] else None
    )
    return scalar


def compose(text, name="workflow"):
    """The single document of text as a Node tree."""
    source = text.encode("utf-8") if isinstance(text, str) else text
    try:
        documents = yaml_json.load_wire(source, name)
    except yaml_json.Refused as error:
        message = str(error)
        if "merge_rejected" in message:
            raise WorkflowError(
                f"{name}: merge keys are not accepted in workflows"
            ) from None
        if "duplicate_key" in message:
            raise WorkflowError(f"{name}: duplicate key: {message}") from None
        raise WorkflowError(f"{name}: not valid YAML: {message}") from None
    if len(documents) != 1 or not isinstance(documents[0], dict):
        raise WorkflowError(f"{name}: a workflow must be exactly one YAML document")
    try:
        return _node(documents[0].get("root"), source)
    except WorkflowError as error:
        raise WorkflowError(f"{name}: {error}") from None


def construct(node):
    """Plain Python values: dict (key order kept), list, str, int, float, bool, None."""
    if isinstance(node, MappingNode):
        out = {}
        for key_node, value_node in node.value:
            key = construct(key_node)
            try:
                hash(key)
            except TypeError:
                raise WorkflowError(
                    "mapping keys must be scalars in workflows"
                ) from None
            if key in out:
                raise WorkflowError(f"duplicate key {key!r}")
            out[key] = construct(value_node)
        return out
    if isinstance(node, SequenceNode):
        return [construct(item) for item in node.value]
    wire = node.wire
    kind = wire["kind"]
    if kind == "string":
        return wire["value"]
    if kind == "int":
        if not re.fullmatch(r"0|-?[1-9][0-9]*", wire["value"]):
            raise ReaderError("yamlprims output has a malformed int")
        return int(wire["value"], 10)
    if kind == "float":
        return _float(wire["bits"])
    if kind == "bool":
        return wire["value"]
    return None


def load_text(text, name="workflow"):
    root = compose(text, name)
    try:
        doc = construct(root)
    except WorkflowError as error:
        raise WorkflowError(f"{name}: {error}") from None
    if not isinstance(doc, dict):
        raise WorkflowError(f"{name}: workflow is not a mapping")
    return doc


def load_dir(directory):
    paths = sorted(pathlib.Path(directory).glob("*.y*ml"))
    if not paths:
        raise WorkflowError("no workflow files found")
    return {
        path.name: (
            load_text(path.read_text(encoding="utf-8"), path.name),
            path.read_text(encoding="utf-8"),
        )
        for path in paths
    }


def jobs(doc):
    value = doc.get("jobs")
    if not isinstance(value, dict):
        raise WorkflowError("workflow has no jobs mapping")
    return value


def steps(job):
    value = job.get("steps") or []
    if not isinstance(value, list):
        raise WorkflowError("steps is not a list")
    return value
