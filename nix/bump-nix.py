import hashlib
import re
import sys
import urllib.request
from pathlib import Path
from xml.etree import ElementTree

LISTING = "https://nix-releases.s3.amazonaws.com/?prefix=nix/nix-&delimiter=/"
PREFIX = "{http://s3.amazonaws.com/doc/2006-03-01/}Prefix"
TARBALL = "https://releases.nixos.org/nix/nix-{v}/nix-{v}-aarch64-linux.tar.xz"
CONTAINERFILE = Path("images/Containerfile")


def latest_version():
    with urllib.request.urlopen(LISTING) as response:
        listing = ElementTree.parse(response)
    versions = []
    for prefix in listing.iter(PREFIX):
        match = re.fullmatch(r"nix/nix-(\d+(?:\.\d+)*)/", prefix.text or "")
        if match:
            versions.append(match.group(1))
    if not versions:
        sys.exit(f"no releases matched in {LISTING}")
    return max(versions, key=lambda v: [int(p) for p in v.split(".")])


def sha256(url):
    digest = hashlib.sha256()
    with urllib.request.urlopen(url) as response:
        for chunk in iter(lambda: response.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def main():
    version = latest_version()
    digest = sha256(TARBALL.format(v=version))
    text = CONTAINERFILE.read_text()
    for arg, value in (("NIX_VERSION", version), ("NIX_SHA256", digest)):
        text, count = re.subn(rf"(?m)^ARG {arg}=.*$", f"ARG {arg}={value}", text)
        if count != 1:
            sys.exit(f"expected 1 ARG {arg} line, found {count}")
    CONTAINERFILE.write_text(text)
    print(f"nix {version} {digest}")


main()
