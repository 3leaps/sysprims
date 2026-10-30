"""Load GitHub Actions workflow files for the workflow lints.

A full YAML parse, so permissions, steps and `uses` are read the way Actions
reads them (quoted keys and scalars, flow style, any indentation). Constructs
that make a value depend on something other than its own text are refused:
anchors, aliases, merge keys, explicit tags and duplicate keys. Workflows here
use none of them; refusing them keeps the lints' view equal to the runner's.
"""

import pathlib
import sys

try:
    import yaml
except ImportError:  # pragma: no cover - environment check
    sys.exit(
        "error: the workflow lints need PyYAML (python3-yaml / pip install pyyaml)"
    )


class WorkflowError(Exception):
    pass


class _Loader(yaml.SafeLoader):
    pass


def _refuse_alias(self, *args, **kwargs):  # noqa: ARG001
    raise WorkflowError("anchors and aliases are not accepted in workflows")


def _mapping(loader, node, deep=False):
    keys = set()
    for key_node, _ in node.value:
        if key_node.tag == "tag:yaml.org,2002:merge":
            raise WorkflowError("merge keys are not accepted in workflows")
        key = loader.construct_object(key_node, deep=deep)
        if key in keys:
            raise WorkflowError(f"duplicate key {key!r}")
        keys.add(key)
    return yaml.SafeLoader.construct_mapping(loader, node, deep=deep)


_Loader.compose_node_original = _Loader.compose_node


def _compose_node(self, parent, index):
    event = self.peek_event()
    if isinstance(event, yaml.AliasEvent) or getattr(event, "anchor", None):
        raise WorkflowError("anchors and aliases are not accepted in workflows")
    tag = getattr(event, "tag", None)
    if tag not in (None, "!") and not str(tag).startswith("tag:yaml.org,2002:"):
        raise WorkflowError(f"explicit tag {tag} is not accepted in workflows")
    return _Loader.compose_node_original(self, parent, index)


_Loader.compose_node = _compose_node
_Loader.add_constructor(yaml.resolver.BaseResolver.DEFAULT_MAPPING_TAG, _mapping)


def load_text(text, name="workflow"):
    try:
        doc = yaml.load(text, Loader=_Loader)  # noqa: S506 - SafeLoader subclass
    except WorkflowError as error:
        raise WorkflowError(f"{name}: {error}") from None
    except yaml.YAMLError as error:
        raise WorkflowError(f"{name}: not valid YAML: {error}") from None
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
