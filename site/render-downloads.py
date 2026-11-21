"""Fill the static landing page with the five archives from a GitHub release."""

import json
import sys
from html import escape
from pathlib import Path

PLATFORMS = {
    "Windows": [
        ("x86_64-pc-windows-msvc.zip", "Intel / AMD (x64)"),
        ("aarch64-pc-windows-msvc.zip", "Windows on Arm"),
    ],
    "macOS": [
        ("aarch64-apple-darwin.tar.gz", "Apple silicon"),
        ("x86_64-apple-darwin.tar.gz", "Intel"),
    ],
    "Linux": [("x86_64-unknown-linux-gnu.tar.gz", "Intel / AMD (x64)")],
}


class IncompleteRelease(ValueError):
    """A required release archive has not finished uploading."""


def render(template, release):
    assets = {asset["name"]: asset for asset in release["assets"]}
    groups = []
    for platform, targets in PLATFORMS.items():
        links = []
        for target, cpu in targets:
            filename = "ruxguitar-" + target
            asset = assets.get(filename)
            if not asset or asset["state"] != "uploaded" or asset["size"] <= 0:
                raise IncompleteRelease(f"{release['tag_name']} is missing a complete {filename}")
            url = escape(asset["browser_download_url"], quote=True)
            archive_type = "ZIP" if target.endswith(".zip") else "tar.gz"
            size = f"{asset['size'] / 1_000_000:.1f} MB"
            links.append(
                f'<a class="archive" href="{url}" '
                f'aria-label="Download {platform} for {cpu}, {size}">'
                f'<span>{cpu}</span><small>{archive_type} · {size}</small></a>'
            )
        if platform == "Linux":
            links.append('<a class="source-link" href="#cargo">Build for another CPU with Cargo</a>')
        groups.append(f'<div id="{platform.lower()}"><h3>{platform}</h3>\n' + "\n".join(links) + "\n</div>")

    linux_archive = assets["ruxguitar-x86_64-unknown-linux-gnu.tar.gz"]
    return (template.replace("<!-- RELEASE_ARCHIVES -->", "\n".join(groups))
            .replace("@LINUX_DOWNLOAD_SIZE@", f"{linux_archive['size'] / 1_000_000:.0f} MB")
            .replace("@RELEASE_URL@", escape(release["html_url"], quote=True))
            .replace("@RELEASE_TAG@", escape(release["tag_name"])))


if __name__ == "__main__":
    template, metadata, output = map(Path, sys.argv[1:])
    try:
        page = render(template.read_text(encoding="utf-8"), json.loads(metadata.read_bytes()))
    except IncompleteRelease as error:
        print(error, file=sys.stderr)
        sys.exit(75)  # EX_TEMPFAIL: Pages retries after the release workflow completes.
    output.write_text(page, encoding="utf-8")
