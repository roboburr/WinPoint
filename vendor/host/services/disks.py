"""disks.* (SPEC §6.16): "Schijven" — disks, partitions and volumes through UDisks2 on the system bus.

* One ``GetManagedObjects`` answers everything. ``parse_managed_objects`` (pure, tested against a recorded machine)
  turns it into Windows-like disks ("Schijf 0 …") with their volumes, unallocated space, SMART summary and the
  guards below. Used/free come from ``statvfs`` of the mount point (bounded by a timeout).
* Live: ``InterfacesAdded``/``InterfacesRemoved`` + ``PropertiesChanged`` of org.freedesktop.UDisks2 and its
  ``NameOwnerChanged`` are followed in an own thread with its own GLib main context; any of them → a fresh
  ``GetManagedObjects`` (debounced 250 ms) → event ``disks.changed {supported, disks, jobs, filesystems, …}`` when
  something changed (hotplug, mount, format, a UDisks job's progress).
* Actions go to UDisks only, with ``ALLOW_INTERACTIVE_AUTHORIZATION`` so polkit (hyprpolkitagent) asks for the
  password. Never sudo / pkexec. A LUKS passphrase goes only into the Unlock / Format call (``redact`` hides it
  in the API log; this module never logs arguments).
* Guards (refused with a Dutch message, also in --dry-run): nothing destructive on a volume that holds ``/``,
  ``/boot``, ``/boot/efi``, ``/home``, ``/usr``, ``/var`` (mounted now or listed in fstab), active swap or the
  running Win11 Shell, nor on the disk that holds one (power off, eject, initialise). A partition that a firmware
  boot entry points at (``/sys/firmware/efi/efivars/Boot####``, e.g. "Windows Boot Manager" on another disk) is
  never formatted, deleted or relabelled and its disk is not initialised. System partitions (EFI, MSR, recovery,
  LDM metadata, BIOS boot) and dynamic-disk data partitions are not deleted one by one (like Schijfbeheer).
  Protection follows LUKS (cleartext → backing device) and device-mapper stacks (sysfs ``slaves``). Destructive
  calls need ``expect {device, size}`` of what the user saw, so a USB stick replaced under the same object path is
  never formatted by mistake. One operation per drive at a time, and nothing is unmounted, locked, ejected or
  powered off while a UDisks job runs on that drive.
* Job-backed calls (format, erase, delete, create, initialise) and polkit-gated calls have no D-Bus timeout: a
  full format of a 22 TB disk takes a day, and the password prompt may wait. If a call does time out while its
  UDisks job still runs, the answer is "still running" (never "failed") and the drive stays locked until the job ends.
* ``WIN11SHELL_DISKS_READONLY=1``: only ``GetManagedObjects`` reaches UDisks (checks against the real system bus).
"""

from __future__ import annotations

import json
import logging
import os
import re
import shutil
import struct
import threading
import time
import uuid as _uuid
from pathlib import Path
from typing import Any, Callable

from .. import ROOT
from ..util import ApiError, dry_for, test_guard_blocks

log = logging.getLogger("win11shell.disks")

PRIORITY = 62

UD = "org.freedesktop.UDisks2"
UD_PATH = "/org/freedesktop/UDisks2"
_P = UD + "."
I_BLOCK, I_PART, I_TABLE, I_FS = _P + "Block", _P + "Partition", _P + "PartitionTable", _P + "Filesystem"
I_DRIVE, I_ATA, I_NVME = _P + "Drive", _P + "Drive.Ata", _P + "NVMe.Controller"
I_CRYPT, I_SWAP, I_LOOP, I_JOB, I_MANAGER = _P + "Encrypted", _P + "Swapspace", _P + "Loop", _P + "Job", _P + "Manager"
OM = "org.freedesktop.DBus.ObjectManager"

MiB = 1024 * 1024
MIN_VOLUME = 8 * MiB
MIN_FREE = MIN_VOLUME             # smaller gaps are alignment leftovers: nothing fits in them
DEBOUNCE_S = 0.25
CALL_TIMEOUT_MS = 120_000         # SMART: a hanging disk must not block a request forever
NO_TIMEOUT_MS = 0x7FFFFFFF        # G_MAXINT: GDBus waits as long as it takes
#: UDisks answers Format / Delete / Create… only when its job is done (a full format of 22 TB ≈ a day) and any call
#: may wait for the polkit password: no wall-clock limit (tests patch it to provoke the "still running" path).
JOB_TIMEOUT_MS = NO_TIMEOUT_MS
JOB_POLL_S = 5.0                  # after a timed-out job call: how often to look whether the job is gone

#: mount point → reason. Volumes holding one of these are never formatted, deleted, unmounted or relabelled.
PROTECTED_MOUNTS = {"/": "root", "/boot": "boot", "/boot/efi": "efi", "/efi": "efi", "/home": "home",
                    "/usr": "usr", "/var": "var"}
#: reasons that block destructive changes (format, delete, label, initialise) but not unmount / safe removal:
#: Ubuntu itself does not need the volume (another OS boots from it).
SOFT_REASONS = frozenset({"boot_entry"})
#: file systems whose tools refuse (or are unsafe) to relabel a mounted volume (ntfslabel: "mounted read-write"):
#: Label wijzigen unmounts, relabels and mounts again. ext2/3/4 relabel online.
OFFLINE_LABEL = frozenset({"ntfs", "exfat", "vfat", "btrfs", "xfs", "f2fs", "nilfs2"})

EFI_VARS = "/sys/firmware/efi/efivars"
EFI_GLOBAL_GUID = "8be4df61-93ca-11d2-aa0d-00e098032b8c"
_BOOT_VAR_RE = re.compile(r"^Boot[0-9A-Fa-f]{4}-" + EFI_GLOBAL_GUID + "$")

_ID_RE = re.compile(r"^/org/freedesktop/UDisks2/(block_devices|drives)/[A-Za-z0-9_]{1,160}$")

_lang: Callable[[], str] = lambda: "nl"

_STR = {
    "nl": {
        "disk": "Schijf {n}",
        "v_root": "Systeemschijf", "v_home": "Persoonlijke bestanden", "v_boot": "Opstartpartitie",
        "v_efi": "EFI-systeempartitie", "v_swap": "Wisselgeheugen", "v_msr": "Gereserveerd door Microsoft",
        "v_recovery": "Herstelpartitie", "v_ldm_meta": "Dynamische schijf (metagegevens)",
        "v_ldm_data": "Dynamische schijf", "v_bios": "BIOS-opstartpartitie", "v_locked": "Versleuteld volume",
        "v_raw": "Niet-geformatteerd", "v_local": "Lokaal station", "v_usb": "USB-station", "v_lvm": "LVM-volume",
        "v_raid": "RAID-onderdeel", "v_extended": "Uitgebreide partitie", "v_disc": "Cd/dvd", "v_loop": "Virtuele schijf",
        "t_efi": "EFI-systeempartitie", "t_basic": "Basisgegevenspartitie", "t_msr": "Gereserveerde partitie van Microsoft",
        "t_recovery": "Herstelpartitie van Windows", "t_ldm_meta": "Metagegevens van een dynamische schijf",
        "t_ldm_data": "Gegevens van een dynamische schijf", "t_linux": "Linux-bestandssysteem",
        "t_linux_root": "Linux-hoofdpartitie", "t_linux_home": "Linux-persoonlijke map", "t_swap": "Linux-wisselgeheugen",
        "t_lvm": "Linux LVM", "t_raid": "Linux RAID", "t_luks": "Linux LUKS (versleuteld)", "t_bios": "BIOS-opstartpartitie",
        "t_xbootldr": "Linux-opstartpartitie", "t_hfs": "Apple HFS+", "t_apfs": "Apple APFS",
        "t_extended": "Uitgebreide partitie", "t_fat": "FAT", "t_ntfs": "NTFS of exFAT", "t_other": "Partitietype {t}",
        "fs_ntfs": "Voor Windows-pc's en grote bestanden", "fs_exfat": "Werkt op bijna alle apparaten (Windows, Mac, Linux)",
        "fs_vfat": "Voor oudere apparaten; bestanden tot 4 GB", "fs_ext4": "Voor Linux (aanbevolen voor Ubuntu)",
        "fs_btrfs": "Linux, met momentopnamen", "fs_xfs": "Linux, voor heel grote bestanden",
        "r_root": "Ubuntu draait vanaf dit volume.", "r_boot": "Dit volume is nodig om de computer op te starten.",
        "r_efi": "Dit volume is nodig om de computer op te starten.", "r_home": "Je persoonlijke map staat op dit volume.",
        "r_usr": "Onderdelen van Ubuntu staan op dit volume.", "r_var": "Onderdelen van Ubuntu staan op dit volume.",
        "r_swap": "Dit volume wordt gebruikt als wisselgeheugen.", "r_shell": "Win11 Shell draait vanaf dit volume.",
        "r_disk": "Ubuntu heeft deze schijf nodig om te werken.",
        "r_boot_entry": "De computer kan vanaf dit volume opstarten ({names}).",
        "r_disk_boot": "Op deze schijf staat een volume waarmee de computer opstart ({names}).",
        "no_format": "Je kunt het daarom niet formatteren.", "no_delete": "Je kunt het daarom niet verwijderen.",
        "no_unmount": "Je kunt het daarom niet ontkoppelen.", "no_label": "Je kunt de naam daarom niet wijzigen.",
        "no_lock": "Je kunt het daarom niet vergrendelen.", "no_eject": "Je kunt hem daarom niet verwijderen.",
        "no_initialize": "Je kunt hem daarom niet initialiseren.",
        "unavailable": "Schijfbeheer werkt niet: UDisks is op deze computer niet beschikbaar.",
        "gone_volume": "Dit volume bestaat niet meer. Misschien is de schijf losgekoppeld.",
        "gone_disk": "Deze schijf bestaat niet meer. Misschien is hij losgekoppeld.",
        "changed": "Deze schijf is veranderd sinds je hem koos. Kijk opnieuw en probeer het nog eens.",
        "expect": "Kies het volume opnieuw en probeer het nog eens.",
        "busy": "‘{name}’ is nog in gebruik. Sluit de bestanden en apps die het gebruiken en probeer het opnieuw.",
        "not_authorized": "Je hebt geen toestemming gegeven. Er is niets veranderd.",
        "cancelled": "Geannuleerd. Er is niets veranderd.",
        "bad_passphrase": "Het wachtwoord klopt niet. Probeer het opnieuw.",
        "failed": "Dat is niet gelukt: {detail}",
        "timeout": "UDisks reageert niet op tijd. Kijk of het toch is gelukt en probeer het zo nodig opnieuw.",
        "readonly": "Alleen-lezen testmodus: er wordt niets aan de schijven veranderd.",
        "fs_missing": "{fs} kan op deze computer niet worden gemaakt: het programma daarvoor ontbreekt.",
        "fs_unknown": "Kies een bestandssysteem.",
        "label_long": "Deze naam is te lang voor {fs} (maximaal {n} tekens).",
        "label_bad": "Deze naam bevat tekens die {fs} niet toestaat.",
        "not_mounted": "Dit volume is niet gekoppeld.",
        "no_fs": "Op dit volume staat geen bestandssysteem dat kan worden gekoppeld.",
        "no_label_fs": "Dit volume heeft geen bestandssysteem met een naam.",
        "not_encrypted": "Dit volume is niet versleuteld.",
        "locked": "Ontgrendel dit volume eerst.",
        "table_full": "Op deze schijf kan geen partitie meer bij: een MBR-schijf heeft er maximaal vier.",
        "no_free": "Deze vrije ruimte bestaat niet meer. Kijk opnieuw en probeer het nog eens.",
        "too_small": "Een volume moet minstens 8 MB groot zijn.",
        "not_partition": "Alleen een partitie kan worden verwijderd. Formatteer dit volume in plaats daarvan.",
        "has_logical": "Verwijder eerst de logische stations in deze partitie.",
        "no_table": "Deze schijf is nog niet geïnitialiseerd. Initialiseer hem eerst.",
        "system_part": "Dit is een systeempartitie ({type}). Die kun je hier niet formatteren of verwijderen. "
                       "Wil je de hele schijf opnieuw gebruiken, kies dan ‘Schijf wissen en initialiseren’.",
        "dynamic_part": "Dit volume hoort bij een dynamische schijf van Windows. Verwijder het in Windows "
                        "(Schijfbeheer), anders raakt Windows de gegevens op deze schijf kwijt.",
        "op_running": "Er loopt al een bewerking op deze schijf. Wacht tot die klaar is.",
        "busy_job": "Er loopt nog een bewerking op ‘{name}’. Wacht tot die klaar is.",
        "still_running": "De bewerking op ‘{name}’ loopt nog. Je ziet de voortgang in Schijven; zet de computer "
                         "niet uit en koppel de schijf niet los.",
        "fstab_label": "Ubuntu koppelt dit volume bij het opstarten aan de hand van zijn naam. "
                       "Daarom kun je de naam hier niet wijzigen.",
        "passphrase": "Kies een wachtwoord van minstens 8 tekens.",
        "passphrase_empty": "Typ het wachtwoord.",
        "no_smart": "Deze schijf geeft geen gezondheidsgegevens door.",
        "bad_table": "Kies GPT of MBR.",
        "bad_args": "Ongeldige invoer.",
        "not_removable": "Deze schijf kan niet worden uitgeworpen.",
    },
    "en": {
        "disk": "Disk {n}",
        "v_root": "System drive", "v_home": "Personal files", "v_boot": "Boot partition",
        "v_efi": "EFI System Partition", "v_swap": "Swap", "v_msr": "Microsoft reserved",
        "v_recovery": "Recovery partition", "v_ldm_meta": "Dynamic disk (metadata)", "v_ldm_data": "Dynamic disk",
        "v_bios": "BIOS boot partition", "v_locked": "Encrypted volume", "v_raw": "Unformatted", "v_local": "Local Disk",
        "v_usb": "USB Drive", "v_lvm": "LVM volume", "v_raid": "RAID member", "v_extended": "Extended partition",
        "v_disc": "CD/DVD", "v_loop": "Virtual disk",
        "t_efi": "EFI System Partition", "t_basic": "Basic data partition", "t_msr": "Microsoft reserved partition",
        "t_recovery": "Windows recovery partition", "t_ldm_meta": "Dynamic disk metadata", "t_ldm_data": "Dynamic disk data",
        "t_linux": "Linux filesystem", "t_linux_root": "Linux root partition", "t_linux_home": "Linux home partition",
        "t_swap": "Linux swap", "t_lvm": "Linux LVM", "t_raid": "Linux RAID", "t_luks": "Linux LUKS (encrypted)",
        "t_bios": "BIOS boot partition", "t_xbootldr": "Linux boot partition", "t_hfs": "Apple HFS+",
        "t_apfs": "Apple APFS", "t_extended": "Extended partition", "t_fat": "FAT", "t_ntfs": "NTFS or exFAT",
        "t_other": "Partition type {t}",
        "fs_ntfs": "For Windows PCs and large files", "fs_exfat": "Works on almost every device (Windows, Mac, Linux)",
        "fs_vfat": "For older devices; files up to 4 GB", "fs_ext4": "For Linux (recommended for Ubuntu)",
        "fs_btrfs": "Linux, with snapshots", "fs_xfs": "Linux, for very large files",
        "r_root": "Ubuntu runs from this volume.", "r_boot": "This volume is needed to start the computer.",
        "r_efi": "This volume is needed to start the computer.", "r_home": "Your personal folder is on this volume.",
        "r_usr": "Parts of Ubuntu are on this volume.", "r_var": "Parts of Ubuntu are on this volume.",
        "r_swap": "This volume is used as swap.", "r_shell": "Win11 Shell runs from this volume.",
        "r_disk": "Ubuntu needs this disk to work.",
        "r_boot_entry": "The computer can start from this volume ({names}).",
        "r_disk_boot": "This disk holds a volume the computer starts from ({names}).",
        "no_format": "That's why it can't be formatted.", "no_delete": "That's why it can't be deleted.",
        "no_unmount": "That's why it can't be unmounted.", "no_label": "That's why its name can't be changed.",
        "no_lock": "That's why it can't be locked.", "no_eject": "That's why it can't be removed.",
        "no_initialize": "That's why it can't be initialised.",
        "unavailable": "Disk management doesn't work: UDisks isn't available on this computer.",
        "gone_volume": "This volume no longer exists. Maybe the disk was disconnected.",
        "gone_disk": "This disk no longer exists. Maybe it was disconnected.",
        "changed": "This disk changed since you chose it. Have another look and try again.",
        "expect": "Choose the volume again and try again.",
        "busy": "‘{name}’ is still in use. Close the files and apps that use it and try again.",
        "not_authorized": "Permission wasn't given. Nothing was changed.",
        "cancelled": "Cancelled. Nothing was changed.",
        "bad_passphrase": "The password is wrong. Please try again.",
        "failed": "That didn't work: {detail}",
        "timeout": "UDisks didn't answer in time. Check whether it worked anyway and try again if needed.",
        "readonly": "Read-only test mode: nothing is changed on the disks.",
        "fs_missing": "{fs} can't be created on this computer: the program for it is missing.",
        "fs_unknown": "Choose a file system.",
        "label_long": "This name is too long for {fs} (at most {n} characters).",
        "label_bad": "This name contains characters {fs} doesn't allow.",
        "not_mounted": "This volume isn't mounted.",
        "no_fs": "This volume has no file system that can be mounted.",
        "no_label_fs": "This volume has no file system with a name.",
        "not_encrypted": "This volume isn't encrypted.",
        "locked": "Unlock this volume first.",
        "table_full": "No more partitions fit on this disk: an MBR disk holds at most four.",
        "no_free": "This free space no longer exists. Have another look and try again.",
        "too_small": "A volume must be at least 8 MB.",
        "not_partition": "Only a partition can be deleted. Format this volume instead.",
        "has_logical": "Delete the logical drives in this partition first.",
        "no_table": "This disk isn't initialised yet. Initialise it first.",
        "system_part": "This is a system partition ({type}). It can't be formatted or deleted here. To reuse the "
                       "whole disk, choose ‘Erase and initialise disk’.",
        "dynamic_part": "This volume belongs to a Windows dynamic disk. Delete it in Windows (Disk Management), "
                        "otherwise Windows loses track of the data on this disk.",
        "op_running": "Another operation is running on this disk. Wait until it has finished.",
        "busy_job": "An operation is still running on ‘{name}’. Wait until it has finished.",
        "still_running": "The operation on ‘{name}’ is still running. You can follow it in Disks; don't turn the "
                         "computer off or unplug the disk.",
        "fstab_label": "Ubuntu mounts this volume at startup by its name. That's why it can't be renamed here.",
        "passphrase": "Choose a password of at least 8 characters.",
        "passphrase_empty": "Type the password.",
        "no_smart": "This disk doesn't report health data.",
        "bad_table": "Choose GPT or MBR.",
        "bad_args": "Invalid input.",
        "not_removable": "This disk can't be ejected.",
    },
}


def _tr(key: str, **kw) -> str:
    table = _STR.get(_lang(), _STR["nl"])
    text = table.get(key) or _STR["nl"].get(key) or key
    try:
        return text.format(**kw) if kw else text
    except (KeyError, IndexError):
        return text


# ---------------------------------------------------------------------------
# tables: partition types, file systems
# ---------------------------------------------------------------------------

GPT_TYPES = {
    "c12a7328-f81f-11d2-ba4b-00a0c93ec93b": ("efi", "system"),
    "ebd0a0a2-b9e5-4433-87c0-68b6b72699c7": ("basic", "data"),
    "e3c9e316-0b5c-4db8-817d-f92df00215ae": ("msr", "system"),
    "de94bba4-06d1-4d40-a16a-bfd50179d6ac": ("recovery", "system"),
    "5808c8aa-7e8f-42e0-85d2-e1e90434cfb3": ("ldm_meta", "system"),
    "af9b60a0-1431-4f62-bc68-3311714a69ad": ("ldm_data", "data"),
    "0fc63daf-8483-4772-8e79-3d69d8477de4": ("linux", "data"),
    "4f68bce3-e8cd-4db1-96e7-fbcaf984b709": ("linux_root", "data"),
    "933ac7e1-2eb4-4f13-b844-0e14e2aef915": ("linux_home", "data"),
    "0657fd6d-a4ab-43c4-84e5-0933c84b4f4f": ("swap", "swap"),
    "e6d6d379-f507-44c2-a23c-238f2a3df928": ("lvm", "data"),
    "a19d880f-05fc-4d3b-a006-743f0f84911e": ("raid", "data"),
    "ca7d7ccb-63ed-4c53-861c-1742536059cc": ("luks", "data"),
    "21686148-6449-6e6f-744e-656564454649": ("bios", "system"),
    "bc13c2ff-59e6-4262-a352-b275fd6f7172": ("xbootldr", "system"),
    "48465300-0000-11aa-aa11-00306543ecac": ("hfs", "data"),
    "7c3457ef-0000-11aa-aa11-00306543ecac": ("apfs", "data"),
}
DOS_TYPES = {
    "0x05": ("extended", "container"), "0x0f": ("extended", "container"), "0x85": ("extended", "container"),
    "0x07": ("ntfs", "data"), "0x0b": ("fat", "data"), "0x0c": ("fat", "data"), "0x0e": ("fat", "data"),
    "0x06": ("fat", "data"), "0x01": ("fat", "data"), "0x04": ("fat", "data"),
    "0x83": ("linux", "data"), "0x82": ("swap", "swap"), "0x8e": ("lvm", "data"), "0xfd": ("raid", "data"),
    "0xef": ("efi", "system"), "0x27": ("recovery", "system"), "0xee": ("other", "system"),
}

#: id → (display name, mkfs programs, label limit, label rule, Linux fs?)
FILESYSTEMS: dict[str, tuple[str, tuple[str, ...], int, str, bool]] = {
    "ntfs": ("NTFS", ("mkfs.ntfs",), 128, "chars", False),
    "exfat": ("exFAT", ("mkfs.exfat",), 15, "chars", False),
    "vfat": ("FAT32", ("mkfs.vfat", "mkfs.fat"), 11, "fat", False),
    "ext4": ("ext4", ("mkfs.ext4",), 16, "bytes", True),
    "btrfs": ("Btrfs", ("mkfs.btrfs",), 255, "bytes", True),
    "xfs": ("XFS", ("mkfs.xfs",), 12, "bytes", True),
}
FS_ORDER = ("ntfs", "exfat", "vfat", "ext4", "btrfs", "xfs")
GPT_FOR_FS = {"ntfs": "ebd0a0a2-b9e5-4433-87c0-68b6b72699c7", "exfat": "ebd0a0a2-b9e5-4433-87c0-68b6b72699c7",
              "vfat": "ebd0a0a2-b9e5-4433-87c0-68b6b72699c7", "ext4": "0fc63daf-8483-4772-8e79-3d69d8477de4",
              "btrfs": "0fc63daf-8483-4772-8e79-3d69d8477de4", "xfs": "0fc63daf-8483-4772-8e79-3d69d8477de4"}
DOS_FOR_FS = {"ntfs": "0x07", "exfat": "0x07", "vfat": "0x0c", "ext4": "0x83", "btrfs": "0x83", "xfs": "0x83"}
_FAT_BAD = set('"*+,./:;<=>?[\\]|')
#: file systems whose label UDisks can change (Filesystem.SetLabel) → (limit, rule)
LABEL_RULES = {fid: (spec[2], spec[3]) for fid, spec in FILESYSTEMS.items()}
LABEL_RULES.update({"ext2": (16, "bytes"), "ext3": (16, "bytes"), "f2fs": (512, "chars"), "nilfs2": (80, "bytes")})

FS_NAMES = {"ntfs": "NTFS", "exfat": "exFAT", "vfat": "FAT32", "ext2": "ext2", "ext3": "ext3", "ext4": "ext4",
            "btrfs": "Btrfs", "xfs": "XFS", "f2fs": "F2FS", "swap": "Wisselgeheugen", "crypto_LUKS": "LUKS",
            "iso9660": "CDFS", "udf": "UDF", "squashfs": "SquashFS", "LVM2_member": "LVM", "hfsplus": "HFS+",
            "apfs": "APFS", "linux_raid_member": "RAID", "BitLocker": "BitLocker", "zfs_member": "ZFS"}


def _sbin_which(name: str) -> str | None:
    path = os.environ.get("PATH", "") + ":/usr/sbin:/sbin:/usr/local/sbin"
    return shutil.which(name, path=path)


def filesystems(supported: list[str] | None, which: Callable[[str], str | None] = _sbin_which) -> list[dict]:
    """The file systems a format / new volume can use: UDisks knows them (Manager.SupportedFilesystems) and the
    mkfs program is installed."""
    out = []
    for fid in FS_ORDER:
        name, progs, limit, rule, linux = FILESYSTEMS[fid]
        known = supported is None or fid in supported
        have = any(which(p) for p in progs)
        out.append({"id": fid, "name": name, "available": bool(known and have), "maxLabel": limit,
                    "labelRule": rule, "linux": linux, "description": _tr(f"fs_{fid}")})
    return out


def check_label(fs_type: str, label: str) -> str:
    """Validated label for ``fs_type`` (raises ApiError). FAT labels are upper-cased like Windows does."""
    if not isinstance(label, str):
        raise ApiError("bad_args", _tr("bad_args"))
    label = label.strip()
    if any(ord(c) < 32 for c in label):
        raise ApiError("bad_label", _tr("label_bad", fs=FS_NAMES.get(fs_type, fs_type)))
    rule_spec = LABEL_RULES.get(fs_type)
    if not rule_spec:
        if len(label) > 64:
            raise ApiError("bad_label", _tr("label_long", fs=FS_NAMES.get(fs_type, fs_type), n=64))
        return label
    limit, rule = rule_spec
    name = FS_NAMES.get(fs_type, fs_type)
    if rule == "fat":
        label = label.upper()
        if any(c in _FAT_BAD for c in label) or any(ord(c) > 127 for c in label):
            raise ApiError("bad_label", _tr("label_bad", fs=name))
        size = len(label)
    elif rule == "bytes":
        size = len(label.encode("utf-8"))
    else:
        size = len(label)
    if size > limit:
        raise ApiError("bad_label", _tr("label_long", fs=name, n=limit))
    return label


# ---------------------------------------------------------------------------
# parsing (pure)
# ---------------------------------------------------------------------------

def _bstr(v: Any) -> str:
    """UDisks ``ay`` (NUL-terminated byte string, unpacked as bytes or a list of ints) → str."""
    if v is None:
        return ""
    if isinstance(v, str):
        return v.rstrip("\0")
    try:
        raw = bytes(v)
    except (TypeError, ValueError):
        return ""
    return raw.rstrip(b"\0").decode("utf-8", "replace")


def _blist(v: Any) -> list[str]:
    if not v:
        return []
    return [s for s in (_bstr(x) for x in v) if s]


def _obj(v: Any) -> str | None:
    s = str(v or "")
    return None if s in ("", "/") else s


def _kelvin(v: Any) -> float | None:
    try:
        k = float(v)
    except (TypeError, ValueError):
        return None
    return round(k - 273.15, 1) if k > 200 else None


def _align_up(v: int, a: int = MiB) -> int:
    return -(-v // a) * a


def _align_down(v: int, a: int = MiB) -> int:
    return (v // a) * a


def sysfs_slaves(devnum: int, sys_root: str = "/sys") -> list[str]:
    """Device names under a device-mapper (or md) block: /sys/dev/block/MAJ:MIN/slaves/*."""
    try:
        d = Path(sys_root) / "dev" / "block" / f"{os.major(devnum)}:{os.minor(devnum)}" / "slaves"
        return sorted(p.name for p in d.iterdir())
    except (OSError, ValueError, OverflowError):
        return []


def free_regions(size: int, table_type: str | None, parts: list[tuple[int, int]], *,
                 container: tuple[int, int] | None = None) -> list[dict]:
    """Unallocated space: gaps ≥ 8 MiB (MiB-aligned) between the top-level partitions (offset, size) of a disk, or —
    with ``container=(offset, size)`` of an MBR extended partition — between its logical partitions. A logical
    region keeps 1 MiB in front for its EBR and 1 MiB at the end as slack (``logical: True``)."""
    if container is not None:
        first, last = container[0], container[0] + container[1]
        lead = tail = MiB
    else:
        if size <= 0:
            return []
        first = 34 * 512 if table_type == "gpt" else 512
        last = size - 33 * 512 if table_type == "gpt" else size
        lead = tail = 0
    out = []

    def gap(a: int, b: int) -> None:
        a, b = _align_up(a) + lead, _align_down(b) - tail
        if b - a >= MIN_FREE:
            out.append({"offset": a, "size": b - a, "logical": container is not None})

    cur = first
    for off, sz in sorted(parts):
        if off - cur >= MIN_FREE:
            gap(cur, off)
        cur = max(cur, off + sz)
    gap(cur, last)
    return out


# ---------------------------------------------------------------------------
# firmware boot entries (read-only: /sys/firmware/efi/efivars/Boot####-…)
# ---------------------------------------------------------------------------

def parse_load_option(raw: bytes) -> dict | None:
    """efivarfs content of a Boot#### variable (4 attribute bytes + EFI_LOAD_OPTION) →
    ``{name, active, partuuids}``: the partitions its device path names (HD() nodes: GPT partition GUID, or for
    MBR "<disk signature>-<partition number>" like blkid's PARTUUID)."""
    if not isinstance(raw, (bytes, bytearray)) or len(raw) < 4 + 6:
        return None
    data = bytes(raw[4:])
    try:
        attrs, fplen = struct.unpack_from("<IH", data, 0)
    except struct.error:
        return None
    end = 6
    while end + 1 < len(data) and data[end:end + 2] != b"\0\0":
        end += 2
    name = data[6:end].decode("utf-16-le", "replace").strip()
    fp = data[end + 2:end + 2 + fplen]
    parts: list[str] = []
    j = 0
    while j + 4 <= len(fp):
        typ, sub, ln = struct.unpack_from("<BBH", fp, j)
        if ln < 4 or j + ln > len(fp):
            break
        if typ == 0x04 and sub == 0x01 and ln >= 42:              # Media / Hard Drive
            number = struct.unpack_from("<I", fp, j + 4)[0]
            sig, sig_type = fp[j + 24:j + 40], fp[j + 41]
            if sig_type == 2:
                parts.append(str(_uuid.UUID(bytes_le=bytes(sig))))
            elif sig_type == 1:
                parts.append(f"{int.from_bytes(sig[:4], 'little'):08x}-{number:02x}")
        if typ == 0x7F and sub == 0xFF:                           # end of the device path
            break
        j += ln
    return {"name": name, "active": bool(attrs & 1), "partuuids": parts}


def efi_boot_entries(root: str = EFI_VARS) -> list[dict]:
    """The firmware's boot entries that name a partition (read-only; [] without UEFI / efivarfs)."""
    out = []
    try:
        names = sorted(n for n in os.listdir(root) if _BOOT_VAR_RE.match(n))
    except OSError:
        return []
    for n in names:
        try:
            with open(os.path.join(root, n), "rb") as f:
                raw = f.read(65536)
        except OSError:
            continue
        e = parse_load_option(raw)
        if e and e["partuuids"]:
            out.append(e)
    return out


class _Blk:
    """One UDisks block object, flattened."""

    __slots__ = ("path", "ifs", "device", "devnum", "size", "id_type", "id_usage", "id_label", "id_uuid", "id_version",
                 "read_only", "hint_ignore", "hint_system", "hint_name", "drive", "crypto_backing", "mounts",
                 "is_fs", "part", "table", "crypt", "swap_active", "loop_file", "config", "hide", "protected",
                 "boot_names")

    def __init__(self, path: str, ifs: dict):
        b = ifs.get(I_BLOCK) or {}
        self.path = path
        self.ifs = ifs
        self.device = _bstr(b.get("PreferredDevice")) or _bstr(b.get("Device"))
        dev = _bstr(b.get("Device"))
        if dev:
            self.device = dev
        try:
            self.devnum = int(b.get("DeviceNumber") or 0)
        except (TypeError, ValueError):
            self.devnum = 0
        self.size = int(b.get("Size") or 0)
        self.id_type = str(b.get("IdType") or "")
        self.id_usage = str(b.get("IdUsage") or "")
        self.id_label = str(b.get("IdLabel") or "")
        self.id_uuid = str(b.get("IdUUID") or "")
        self.id_version = str(b.get("IdVersion") or "")
        self.read_only = bool(b.get("ReadOnly"))
        self.hint_ignore = bool(b.get("HintIgnore"))
        self.hint_system = bool(b.get("HintSystem"))
        self.hint_name = str(b.get("HintName") or "")
        self.drive = _obj(b.get("Drive"))
        self.crypto_backing = _obj(b.get("CryptoBackingDevice"))
        fs = ifs.get(I_FS)
        self.is_fs = fs is not None
        self.mounts = _blist(fs.get("MountPoints")) if fs else []
        self.part = ifs.get(I_PART)
        self.table = ifs.get(I_TABLE)
        self.crypt = ifs.get(I_CRYPT)
        sw = ifs.get(I_SWAP)
        self.swap_active = bool(sw and sw.get("Active"))
        lp = ifs.get(I_LOOP)
        self.loop_file = _bstr(lp.get("BackingFile")) if lp else None
        self.config = []
        for item in b.get("Configuration") or []:
            try:
                kind, data = item
                self.config.append((str(kind), {k: (_bstr(v) if isinstance(v, (bytes, list)) else v)
                                                for k, v in (data or {}).items()}))
            except (TypeError, ValueError):
                continue
        opts = [str(o) for o in (b.get("UserspaceMountOptions") or [])]
        self.hide = "x-gdu.hide" in opts or "x-udisks2.hide" in opts
        self.protected: list[str] = []
        self.boot_names: list[str] = []


def _drive_kind(drive: dict, has_nvme: bool, device: str) -> str:
    media = str(drive.get("Media") or "")
    compat = [str(m) for m in drive.get("MediaCompatibility") or []]
    bus = str(drive.get("ConnectionBus") or "")
    if drive.get("Optical") or media.startswith("optical") or any(m.startswith("optical") for m in compat):
        return "optical"
    if has_nvme or os.path.basename(device).startswith("nvme"):
        return "nvme"
    if bus == "sdio" or os.path.basename(device).startswith("mmcblk") or media.startswith("flash_sd") or \
            (drive.get("MediaRemovable") and any(m.startswith("flash_sd") for m in compat)):
        return "sd"
    if bus == "usb":
        return "usb"
    rate = drive.get("RotationRate")
    try:
        rate = int(rate)
    except (TypeError, ValueError):
        rate = -1
    return "ssd" if rate == 0 else "hdd"


def smart_summary(drive_ifs: dict) -> dict:
    """SMART / NVMe health from the drive's properties only (no method calls)."""
    nv = drive_ifs.get(I_NVME)
    if nv is not None:
        updated = int(nv.get("SmartUpdated") or 0)
        warnings = [str(w) for w in nv.get("SmartCriticalWarning") or []]
        poh = int(nv.get("SmartPowerOnHours") or 0)
        temp = _kelvin(nv.get("SmartTemperature"))
        if not updated:
            status = "unknown"
        elif any(w in ("spare", "degraded", "readonly", "volatile_mem", "pmr_readonly") for w in warnings):
            status = "critical"
        elif warnings:
            status = "warning"
        else:
            status = "ok"
        return {"supported": True, "kind": "nvme", "enabled": True, "updated": updated,
                "healthy": None if status == "unknown" else status == "ok", "status": status,
                "temperature": temp, "powerOnHours": poh or None, "criticalWarnings": warnings,
                "failingAttributes": 0, "badSectors": None, "percentUsed": None,
                "selftest": str(nv.get("SmartSelftestStatus") or "") or None}
    ata = drive_ifs.get(I_ATA)
    if ata is not None and ata.get("SmartSupported"):
        updated = int(ata.get("SmartUpdated") or 0)
        enabled = bool(ata.get("SmartEnabled"))
        failing = bool(ata.get("SmartFailing"))
        nfail = int(ata.get("SmartNumAttributesFailing") if ata.get("SmartNumAttributesFailing") is not None else -1)
        past = int(ata.get("SmartNumAttributesFailedInThePast")
                   if ata.get("SmartNumAttributesFailedInThePast") is not None else -1)
        bad = int(ata.get("SmartNumBadSectors") if ata.get("SmartNumBadSectors") is not None else -1)
        secs = int(ata.get("SmartPowerOnSeconds") or 0)
        if not updated or not enabled:
            status = "unknown"
        elif failing or nfail > 0:
            status = "critical"
        elif bad > 0 or past > 0:
            status = "warning"
        else:
            status = "ok"
        return {"supported": True, "kind": "ata", "enabled": enabled, "updated": updated,
                "healthy": None if status == "unknown" else status == "ok", "status": status,
                "temperature": _kelvin(ata.get("SmartTemperature")), "powerOnHours": (secs // 3600) or None,
                "criticalWarnings": [], "failingAttributes": max(nfail, 0), "failedInPast": max(past, 0),
                "badSectors": bad if bad >= 0 else None, "percentUsed": None,
                "selftest": str(ata.get("SmartSelftestStatus") or "") or None}
    return {"supported": False, "status": "unknown", "healthy": None}


def _part_type(part: dict | None, table_type: str | None) -> tuple[str, str, str]:
    """→ (type key, role 'system'|'data'|'swap'|'container', raw type)."""
    if not part:
        return ("", "data", "")
    raw = str(part.get("Type") or "").lower()
    if table_type == "dos" or raw.startswith("0x"):
        key, role = DOS_TYPES.get(raw, ("other", "data"))
        if part.get("IsContainer"):
            key, role = "extended", "container"
        return key, role, raw
    key, role = GPT_TYPES.get(raw, ("other", "data"))
    return key, role, raw


def _usage_of(statvfs: Callable[[str], Any] | None, mount: str | None) -> tuple[int | None, int | None, int | None]:
    if not mount or statvfs is None:
        return None, None, None
    try:
        st = statvfs(mount)
    except (OSError, TimeoutError, ValueError):
        return None, None, None
    if st is None:
        return None, None, None
    try:
        total = int(st.f_blocks) * int(st.f_frsize)
        free = int(st.f_bavail) * int(st.f_frsize)
        used = total - int(st.f_bfree) * int(st.f_frsize)
    except (AttributeError, TypeError, ValueError):
        return None, None, None
    if total <= 0:
        return None, None, None
    return total, max(0, used), max(0, free)


def _longest_mount(blocks: dict[str, _Blk], path: str) -> _Blk | None:
    best, best_len = None, -1
    for b in blocks.values():
        for m in b.mounts:
            mm = m.rstrip("/") or "/"
            if (path == mm or path.startswith(mm + "/") or mm == "/") and len(mm) > best_len:
                best, best_len = b, len(mm)
    return best


def hard_reasons(reasons: list[str]) -> list[str]:
    """The protection reasons Ubuntu itself depends on (everything except another OS's boot volume)."""
    return [r for r in reasons or [] if r not in SOFT_REASONS]


def compute_protection(blocks: dict[str, _Blk], *, protected_mounts: dict[str, str] = PROTECTED_MOUNTS,
                       shell_paths: tuple[str, ...] = (), swap_files: tuple[str, ...] = (),
                       slaves_of: Callable[[int], list[str]] | None = sysfs_slaves,
                       boot_entries: list[dict] | tuple = ()) -> None:
    """Fill ``_Blk.protected`` (reasons) for every block that must never be formatted, deleted or unmounted:
    mounted (or fstab-listed) at a system mount point, active swap, the shell's own install, or named by a firmware
    boot entry (``boot_entry``: a soft reason, see SOFT_REASONS)."""
    def mark(b: _Blk | None, reason: str) -> bool:
        if b is None or reason in b.protected:
            return False
        b.protected.append(reason)
        return True

    boot_by_uuid: dict[str, list[str]] = {}
    for e in boot_entries or ():
        for u in e.get("partuuids") or []:
            names = boot_by_uuid.setdefault(str(u).lower(), [])
            if e.get("name") and e["name"] not in names:
                names.append(e["name"])
    for b in blocks.values():
        for m in b.mounts:
            mm = m.rstrip("/") or "/"
            if mm in protected_mounts:
                mark(b, protected_mounts[mm])
        # fstab lists it at /boot, /boot/efi … although it is not mounted right now (noauto, automount, idle timeout)
        for kind, c in b.config:
            if kind == "fstab":
                mm = str(c.get("dir") or "").rstrip("/") or "/"
                if str(c.get("dir") or "").startswith("/") and mm in protected_mounts:
                    mark(b, protected_mounts[mm])
        if b.swap_active:
            mark(b, "swap")
        puuid = str((b.part or {}).get("UUID") or "").lower()
        if puuid and puuid in boot_by_uuid:
            mark(b, "boot_entry")
            for n in boot_by_uuid[puuid] or ["?"]:
                if n not in b.boot_names:
                    b.boot_names.append(n)
    for p in shell_paths:
        if p:
            mark(_longest_mount(blocks, p), "shell")
    for p in swap_files:
        if p:
            mark(_longest_mount(blocks, p), "swap")
    by_dev = {b.device: b for b in blocks.values() if b.device}
    changed = True
    rounds = 0
    while changed and rounds < 8:
        changed = False
        rounds += 1
        for b in list(blocks.values()):
            if not b.protected:
                continue
            under = []
            if b.crypto_backing:
                under.append(blocks.get(b.crypto_backing))
            if slaves_of is not None and b.devnum and not b.part and (b.drive is None or b.crypto_backing):
                for name in slaves_of(b.devnum) or []:
                    under.append(by_dev.get("/dev/" + name))
            for u in under:
                for r in list(b.protected):
                    if mark(u, r):
                        changed = True


def _volume_name(b: _Blk, clear: _Blk | None, ptype: str, kind: str, disk_kind: str) -> str:
    label = (clear.id_label if clear else "") or b.id_label or b.hint_name
    if label:
        return label
    mounts = (clear.mounts if clear else []) or b.mounts
    reasons = (clear.protected if clear else []) + b.protected
    if "/" in mounts or "root" in reasons:
        return _tr("v_root")
    if "/home" in mounts:
        return _tr("v_home")
    if any(m in ("/boot/efi", "/efi") for m in mounts) or ptype == "efi":
        return _tr("v_efi")
    if "/boot" in mounts or ptype == "xbootldr":
        return _tr("v_boot")
    fst = (clear.id_type if clear else "") or b.id_type
    if fst == "swap" or ptype == "swap":
        return _tr("v_swap")
    special = {"msr": "v_msr", "recovery": "v_recovery", "ldm_meta": "v_ldm_meta", "ldm_data": "v_ldm_data",
               "bios": "v_bios", "extended": "v_extended", "lvm": "v_lvm", "raid": "v_raid"}
    if ptype in special:
        return _tr(special[ptype])
    if b.crypt is not None and clear is None:
        return _tr("v_locked")
    if fst == "LVM2_member":
        return _tr("v_lvm")
    if fst in ("iso9660", "udf") and disk_kind == "optical":
        return _tr("v_disc")
    if not fst:
        return _tr("v_raw")
    return _tr("v_usb") if disk_kind in ("usb", "sd") else _tr("v_local")


def _volume(b: _Blk, blocks: dict[str, _Blk], cleartext_of: dict[str, _Blk], table_type: str | None,
            disk_kind: str, statvfs, jobs_by_obj: dict[str, dict]) -> dict:
    clear = cleartext_of.get(b.path)
    ptype, role, raw = _part_type(b.part, table_type)
    fsb = clear if clear is not None else b
    mounts = list(fsb.mounts)
    total, used, free = _usage_of(statvfs, mounts[0] if mounts else None)
    protected = list(dict.fromkeys(b.protected + (clear.protected if clear else [])))
    fs_type = fsb.id_type if (fsb.id_usage in ("filesystem", "other", "raid", "crypto") or fsb.id_type) else ""
    encrypted = None
    if b.crypt is not None or b.id_type == "crypto_LUKS":
        encrypted = {"type": (b.id_type or "crypto") + (f" {b.id_version}" if b.id_version else ""),
                     "unlocked": clear is not None, "cleartext": clear.path if clear else None,
                     "cleartextDevice": clear.device if clear else None,
                     "inner": clear.id_type if clear else None}
        if clear is None:
            fs_type = "crypto_LUKS"
    is_swap = fs_type == "swap" or ptype == "swap"
    swap_active = b.swap_active or bool(clear and clear.swap_active)
    fstab = [c for kind, c in (b.config + (clear.config if clear else [])) if kind == "fstab"]
    fstab_by_label = any(str(c.get("fsname", "")).startswith(("LABEL=", "/dev/disk/by-label/")) for c in fstab)
    part = b.part or {}
    job = jobs_by_obj.get(b.path) or (jobs_by_obj.get(clear.path) if clear else None)
    can_fs = fsb.is_fs
    hard = hard_reasons(protected)
    boot_names = list(dict.fromkeys(b.boot_names + (clear.boot_names if clear else [])))
    # Partition.Size: the kernel shows an MBR extended partition's block as 1 KiB
    size = int(part.get("Size") or 0) or b.size if b.part else b.size
    vol = {
        "id": b.path, "device": b.device, "label": (clear.id_label if clear else b.id_label) or "",
        "name": _volume_name(b, clear, ptype, "partition" if b.part else "whole", disk_kind),
        "fsType": fs_type or None, "fsName": FS_NAMES.get(fs_type, fs_type.upper() if fs_type else None),
        "fsVersion": (fsb.id_version or None) if fs_type else None, "uuid": fsb.id_uuid or None,
        "size": size, "offset": int(part.get("Offset") or 0), "kind": "partition" if b.part else "whole",
        "partNumber": int(part.get("Number") or 0) or None, "partType": raw or None,
        "partTypeKey": ptype or None, "partTypeName": _tr(f"t_{ptype}", t=raw) if ptype else None,
        "partName": str(part.get("Name") or "") or None, "role": role,
        "isContainer": bool(part.get("IsContainer")), "isContained": bool(part.get("IsContained")),
        "mountPoints": mounts, "mounted": bool(mounts), "mountPoint": mounts[0] if mounts else None,
        "used": used, "free": free, "total": total,
        "isSystem": bool(hard), "protected": protected, "bootEntries": boot_names,
        "isSwap": is_swap, "swapActive": swap_active,
        "encrypted": encrypted, "readOnly": b.read_only, "hidden": b.hint_ignore,
        "fstab": bool(fstab), "fstabByLabel": fstab_by_label,
        "canMount": bool(can_fs and not mounts), "canUnmount": bool(can_fs and mounts and not hard),
        "canLabel": bool(can_fs and fs_type in LABEL_RULES and not protected and not b.read_only and not fstab_by_label),
        # ntfslabel & co. refuse a mounted volume: Label wijzigen unmounts it and mounts it again
        "labelRemount": bool(mounts and fs_type in OFFLINE_LABEL),
        "canFormat": not protected and not b.read_only and role not in ("system", "container"),
        # like Schijfbeheer: system partitions and dynamic-disk volumes are not deleted one by one
        "canDelete": bool(b.part) and not protected and not b.read_only and role != "system" and ptype != "ldm_data",
        "busy": {"operation": job["operation"], "progress": job["progress"]} if job else None,
    }
    return vol


def parse_managed_objects(objs: dict, *, statvfs: Callable[[str], Any] | None = None,
                          protected_mounts: dict[str, str] = PROTECTED_MOUNTS, shell_paths: tuple[str, ...] = (),
                          swap_files: tuple[str, ...] = (), slaves_of: Callable[[int], list[str]] | None = sysfs_slaves,
                          which: Callable[[str], str | None] = _sbin_which,
                          boot_entries: list[dict] | tuple = ()) -> dict:
    """GetManagedObjects (unpacked) → ``{supported, disks, jobs, filesystems, encryption, version}`` (SPEC §6.16).
    ``boot_entries``: ``efi_boot_entries()`` (firmware Boot#### variables), for the ``boot_entry`` guard."""
    blocks: dict[str, _Blk] = {}
    drives: dict[str, dict] = {}
    jobs: list[dict] = []
    manager: dict = {}
    for path, ifs in (objs or {}).items():
        path = str(path)
        if I_BLOCK in ifs:
            blocks[path] = _Blk(path, ifs)
        if I_DRIVE in ifs:
            drives[path] = ifs
        if I_JOB in ifs:
            j = ifs[I_JOB]
            prog = float(j.get("Progress") or 0.0) if j.get("ProgressValid") else None
            jobs.append({"id": path, "operation": str(j.get("Operation") or ""), "progress": prog,
                         "objects": [str(o) for o in j.get("Objects") or []], "cancelable": bool(j.get("Cancelable")),
                         "rate": int(j.get("Rate") or 0) or None, "expectedEnd": int(j.get("ExpectedEndTime") or 0) or None})
        if I_MANAGER in ifs:
            manager = ifs[I_MANAGER]
    jobs.sort(key=lambda j: j["id"])
    jobs_by_obj: dict[str, dict] = {}
    for j in jobs:
        for o in j["objects"]:
            jobs_by_obj.setdefault(o, j)

    compute_protection(blocks, protected_mounts=protected_mounts, shell_paths=shell_paths, swap_files=swap_files,
                       slaves_of=slaves_of, boot_entries=boot_entries)
    cleartext_of = {b.crypto_backing: b for b in blocks.values() if b.crypto_backing}

    # whole-disk blocks: not a partition, not a cleartext / device-mapper child
    disks: list[dict] = []
    wholes: list[tuple[str, _Blk, dict | None]] = []
    for b in blocks.values():
        if b.part is not None or b.crypto_backing:
            continue
        if b.drive and b.drive in drives:
            wholes.append((str(drives[b.drive][I_DRIVE].get("SortKey") or b.drive), b, drives[b.drive]))
        elif b.loop_file is not None or b.ifs.get(I_LOOP) is not None:
            if b.hint_ignore or b.hide or b.size == 0 or (b.loop_file or "").startswith("/var/lib/snapd/") or \
                    any(m.startswith("/snap/") for m in b.mounts):
                continue
            wholes.append(("zz-loop/" + b.device, b, None))
    wholes.sort(key=lambda x: x[0])
    parts_of: dict[str, list[_Blk]] = {}
    for b in blocks.values():
        if b.part is not None:
            parts_of.setdefault(str(b.part.get("Table") or ""), []).append(b)

    for n, (_k, w, drive_ifs) in enumerate(wholes):
        drive = (drive_ifs or {}).get(I_DRIVE) or {}
        kind = "loop" if drive_ifs is None else _drive_kind(drive, I_NVME in (drive_ifs or {}), w.device)
        table_type = str(w.table.get("Type") or "") or None if w.table is not None else None
        media = bool(drive.get("MediaAvailable", True)) if drive_ifs is not None else True
        volumes: list[dict] = []
        free: list[dict] = []
        primaries = 0
        if w.table is not None:
            plist = sorted(parts_of.get(w.path, []), key=lambda p: int(p.part.get("Offset") or 0))
            for p in plist:
                volumes.append(_volume(p, blocks, cleartext_of, table_type, kind, statvfs, jobs_by_obj))
            span = lambda p: (int(p.part.get("Offset") or 0), int(p.part.get("Size") or p.size))  # noqa: E731
            top = [span(p) for p in plist if not p.part.get("IsContained")]
            primaries = len(top)
            full = table_type == "dos" and primaries >= 4
            free = [dict(f, canCreate=not w.read_only and not full, container=None)
                    for f in free_regions(w.size, table_type, top)]
            # free space inside an MBR extended partition: room for logical drives (no limit of four)
            for c in (p for p in plist if p.part.get("IsContainer")):
                cspan = span(c)
                inside = [span(p) for p in plist if p.part.get("IsContained")
                          and cspan[0] <= span(p)[0] < cspan[0] + cspan[1]]
                free += [dict(f, canCreate=not w.read_only, container=c.path)
                         for f in free_regions(w.size, table_type, inside, container=cspan)]
            free.sort(key=lambda f: f["offset"])
            if any(v["isContained"] for v in volumes):        # an extended partition with logical drives in it
                for v in volumes:
                    if v["isContainer"]:
                        v["canDelete"] = False
        elif w.size > 0 and (w.id_usage or w.id_type or w.crypt is not None):
            volumes.append(_volume(w, blocks, cleartext_of, None, kind, statvfs, jobs_by_obj))
        elif w.size > 0 and media:
            free = [{"offset": 0, "size": w.size, "logical": False, "canCreate": False, "container": None}]
        reasons: list[str] = []
        boot_names: list[str] = []
        for v in volumes:
            for r in v["protected"]:
                if r not in reasons:
                    reasons.append(r)
            boot_names += [x for x in v["bootEntries"] if x not in boot_names]
        for r in w.protected:
            if r not in reasons:
                reasons.append(r)
        hard = hard_reasons(reasons)
        vendor = str(drive.get("Vendor") or "").strip()
        model = str(drive.get("Model") or "").strip()
        title = " ".join(x for x in (vendor, model) if x) or (os.path.basename(w.loop_file) if w.loop_file else "")
        # busy: a job on the disk / drive itself, else the first one on any of its volumes (a partition being formatted)
        job = jobs_by_obj.get(w.path) or (jobs_by_obj.get(w.drive) if w.drive else None) or \
            next((v["busy"] for v in volumes if v["busy"]), None)
        disk = {
            # the whole block is the disk: NVMe namespaces (nvme0n1, nvme0n2) and multipath members share one Drive
            "id": w.path, "drive": w.drive if drive_ifs is not None else None,
            "block": w.path, "device": w.device, "number": n,
            "name": _tr("disk", n=n), "model": title or w.device, "vendor": vendor or None,
            "serial": str(drive.get("Serial") or "") or None, "revision": str(drive.get("Revision") or "") or None,
            "wwn": str(drive.get("WWN") or "") or None, "size": w.size or int(drive.get("Size") or 0),
            "kind": kind, "bus": str(drive.get("ConnectionBus") or "") or ("nvme" if kind == "nvme" else None),
            "rotationRate": int(drive.get("RotationRate") or 0) if drive_ifs is not None else None,
            "removable": bool(drive.get("Removable")) or kind in ("usb", "sd", "optical"),
            "mediaRemovable": bool(drive.get("MediaRemovable")), "media": media,
            "ejectable": bool(drive.get("Ejectable")), "canPowerOff": bool(drive.get("CanPowerOff")),
            "readOnly": w.read_only, "loopFile": w.loop_file,
            "isSystem": bool(hard), "protected": reasons, "bootEntries": boot_names,
            "canEject": not hard, "canInitialize": not reasons and not w.read_only and kind != "optical",
            "smart": smart_summary(drive_ifs or {}),
            "partitionTable": table_type, "initialized": w.table is not None or bool(volumes),
            "canPartition": w.table is not None and not w.read_only and any(f["canCreate"] for f in free),
            "tableFull": table_type == "dos" and primaries >= 4,
            "dynamic": any(v["partTypeKey"] in ("ldm_meta", "ldm_data") for v in volumes),
            "volumes": volumes, "free": free,
            "busy": {"operation": job["operation"], "progress": job["progress"]} if job else None,
        }
        disks.append(disk)
    supported = list(manager.get("SupportedFilesystems") or []) if manager else None
    return {
        "supported": True, "disks": disks, "jobs": jobs,
        "filesystems": filesystems(supported or None, which),
        "encryption": {"supported": bool(manager.get("SupportedEncryptionTypes")) if manager else False,
                       "types": [str(t) for t in manager.get("SupportedEncryptionTypes") or []] if manager else [],
                       "default": str(manager.get("DefaultEncryptionType") or "luks2") if manager else "luks2"},
        "version": str(manager.get("Version") or "") or None,
    }


# ---------------------------------------------------------------------------
# usage with a timeout (a dying USB disk can block statvfs)
# ---------------------------------------------------------------------------

class _StatCache:
    def __init__(self, ttl: float = 3.0, timeout: float = 1.0):
        self.ttl = ttl
        self.timeout = timeout
        self._cache: dict[str, tuple[float, Any]] = {}
        self._lock = threading.Lock()

    def __call__(self, path: str):
        now = time.monotonic()
        with self._lock:
            hit = self._cache.get(path)
            if hit and now - hit[0] < self.ttl:
                return hit[1]
        box: dict[str, Any] = {}

        def work():
            try:
                box["v"] = os.statvfs(path)
            except OSError as e:
                box["e"] = e

        t = threading.Thread(target=work, name="disks-statvfs", daemon=True)
        t.start()
        t.join(self.timeout)
        v = box.get("v")
        with self._lock:
            self._cache[path] = (now, v)
        return v


# ---------------------------------------------------------------------------
# the service
# ---------------------------------------------------------------------------

_gi_mods = None


def _gi():
    global _gi_mods
    if _gi_mods is None:
        import gi
        gi.require_version("Gio", "2.0")
        from gi.repository import Gio, GLib
        _gi_mods = (Gio, GLib)
    return _gi_mods


def _swap_files() -> tuple[str, ...]:
    try:
        lines = Path("/proc/swaps").read_text().splitlines()[1:]
    except OSError:
        return ()
    return tuple(ln.split()[0].replace("\\040", " ") for ln in lines if ln.split() and ln.split()[1] == "file")


def _remote_error(e: Exception) -> tuple[str, str]:
    """(D-Bus error name, message without the remote-error prefix)."""
    msg = str(getattr(e, "message", None) or e)
    m = re.search(r"GDBus\.Error:([A-Za-z0-9_.]+):\s*(.*)$", msg, re.S)
    if m:
        return m.group(1), m.group(2).strip()
    name = ""
    try:
        Gio, _ = _gi()
        if Gio.DBusError.is_remote_error(e):
            name = Gio.DBusError.get_remote_error(e) or ""
    except Exception:  # noqa: BLE001
        pass
    if not name:
        m = re.search(r"(org\.freedesktop\.[A-Za-z0-9_.]+)", msg)
        name = m.group(1) if m else ""
    return name, msg.strip()


def map_error(e: Exception, name: str = "") -> ApiError:
    """UDisks / D-Bus error → ApiError with a Dutch message (never contains the call's arguments)."""
    if isinstance(e, ApiError):
        return e
    ename, msg = _remote_error(e)
    low = msg.lower()
    short = ename.rsplit(".", 1)[-1]
    if ename.endswith("NotAuthorizedDismissed") or short == "Cancelled" or "dismissed" in low:
        return ApiError("cancelled", _tr("cancelled"))
    if short.startswith("NotAuthorized") or "not authorized" in low:
        return ApiError("not_authorized", _tr("not_authorized"))
    if short == "DeviceBusy" or "target is busy" in low or "device or resource busy" in low or " busy" in low:
        return ApiError("busy", _tr("busy", name=name or "?"))
    if "passphrase" in low or "no key available" in low or "operation not permitted" in low and "unlock" in low:
        return ApiError("bad_passphrase", _tr("bad_passphrase"))
    if short in ("ServiceUnknown", "NameHasNoOwner") or "was not provided by any .service" in low:
        return ApiError("unavailable", _tr("unavailable"))
    if short in ("UnknownObject", "UnknownMethod", "UnknownInterface") or "no such interface" in low or \
            "object does not exist" in low:
        return ApiError("not_found", _tr("gone_volume"))
    if short in ("Timedout", "Timeout", "NoReply") or "timeout" in low or "timed out" in low:
        return ApiError("timeout", _tr("timeout"))
    if short == "NotMounted":
        return ApiError("not_mounted", _tr("not_mounted"))
    detail = re.sub(r"\s+", " ", msg)[:240] or short or type(e).__name__
    return ApiError("failed", _tr("failed", detail=detail))


class _DriveOps:
    """The drives an operation is running on (format, delete, unmount, power off …): one per drive at a time."""

    def __init__(self):
        self._lock = threading.Lock()
        self._held: set[str] = set()

    def acquire(self, keys: tuple[str, ...]) -> bool:
        with self._lock:
            if any(k in self._held for k in keys):
                return False
            self._held.update(keys)
            return True

    def release(self, keys: tuple[str, ...]) -> None:
        with self._lock:
            self._held.difference_update(keys)

    def held(self, key: str | None = None) -> bool:
        with self._lock:
            return bool(self._held) if key is None else key in self._held


class _Op:
    """``with svc._operation(keys) as op:`` — refuses (op_running) when one of the drives is busy already."""

    def __init__(self, svc: "DisksService", keys):
        self.svc = svc
        self.keys = tuple(sorted(k for k in keys if k))
        self.handed_off = False

    def __enter__(self):
        if not self.svc._ops.acquire(self.keys):
            raise ApiError("op_running", _tr("op_running"))
        return self

    def __exit__(self, *exc):
        if not self.handed_off:
            self.svc._ops.release(self.keys)
        return False

    def hand_off(self, done: Callable[[], bool]) -> None:
        """The call returned but its UDisks job still runs: keep the drives locked until ``done()``."""
        self.handed_off = True
        svc, keys = self.svc, self.keys

        def wait():
            try:
                while True:
                    time.sleep(JOB_POLL_S)
                    try:
                        if done():
                            break
                    except Exception:  # noqa: BLE001 - UDisks gone: its jobs are gone as well
                        break
            finally:
                svc._ops.release(keys)
                svc._done()
        threading.Thread(target=wait, name="disks-job-wait", daemon=True).start()


class DisksService:
    """UDisks2 client. ``bus``: a Gio.DBusConnection (tests: a private bus with a fake UDisks2); default the system
    bus, opened on first use."""

    def __init__(self, bus=None, *, emit: Callable[[str, Any], Any] | None = None,
                 statvfs: Callable[[str], Any] | None = None, which: Callable[[str], str | None] = _sbin_which,
                 slaves_of: Callable[[int], list[str]] | None = sysfs_slaves, shell_paths: tuple[str, ...] | None = None,
                 swap_files: Callable[[], tuple[str, ...]] = _swap_files, readonly: bool | None = None,
                 open_app: Callable[..., Any] | None = None,
                 efi_boot: Callable[[], list[dict]] | None = efi_boot_entries):
        self._bus = bus
        self._bus_lock = threading.Lock()
        self.emit = emit or (lambda *_a: None)
        self.statvfs = statvfs if statvfs is not None else _StatCache()
        self.which = which
        self.slaves_of = slaves_of
        self.shell_paths = shell_paths if shell_paths is not None else (os.path.realpath(str(ROOT)),)
        self.swap_files = swap_files
        self.readonly = (os.environ.get("WIN11SHELL_DISKS_READONLY") == "1") if readonly is None else bool(readonly)
        self.open_app = open_app
        self.efi_boot = efi_boot
        self._boot: tuple[float, list[dict]] = (-1e9, [])
        self._lock = threading.RLock()
        self._snap: dict | None = None
        self._snap_t = 0.0
        self._last_sig: str | None = None
        self._watch: _Watcher | None = None
        self._ops = _DriveOps()

    # -- bus -------------------------------------------------------------------
    def bus(self):
        with self._bus_lock:
            if self._bus is None:
                Gio, _ = _gi()
                self._bus = Gio.bus_get_sync(Gio.BusType.SYSTEM, None)
            return self._bus

    def _call(self, path: str, iface: str, method: str, sig: str = "()", args: tuple = (), reply: str | None = None,
              timeout_ms: int | None = None, name: str = "") -> Any:
        """One UDisks method call (polkit may ask for the password: no time limit unless ``timeout_ms``). Raises
        ApiError."""
        read = (iface == OM and method == "GetManagedObjects")
        if self.readonly and not read:
            raise ApiError("readonly", _tr("readonly"))
        Gio, GLib = _gi()
        try:
            params = GLib.Variant(sig, args) if sig != "()" or args else None
            res = self.bus().call_sync(UD, path, iface, method, params,
                                       GLib.VariantType.new(reply) if reply else None,
                                       Gio.DBusCallFlags.ALLOW_INTERACTIVE_AUTHORIZATION,
                                       JOB_TIMEOUT_MS if timeout_ms is None else timeout_ms, None)
        except ApiError:
            raise
        except Exception as e:  # noqa: BLE001 - GLib.Error and friends
            raise map_error(e, name) from None
        return res.unpack() if res is not None else None

    def _job_call(self, op: _Op, objects: set[str], path: str, iface: str, method: str, sig: str, args: tuple,
                  reply: str | None = None, name: str = "") -> Any:
        """A call UDisks answers when its job is done. If it times out while the job still runs: "still running"
        (never "failed"), and the drive stays locked until the job is gone."""
        try:
            return self._call(path, iface, method, sig, args, reply=reply, timeout_ms=JOB_TIMEOUT_MS, name=name)
        except ApiError as e:
            if e.code != "timeout":
                raise
            try:
                running = self._jobs_on(self.snapshot(refresh=True), objects)
            except ApiError:
                running = []
            if not running:
                raise
            op.hand_off(lambda: not self._jobs_on(self.snapshot(refresh=True), objects))
            raise ApiError("still_running", _tr("still_running", name=name)) from None

    @staticmethod
    def _opts(d: dict) -> dict:
        """Python dict → a{sv} contents (bool → b, int → t, str → s)."""
        _, GLib = _gi()
        out = {}
        for k, v in d.items():
            if isinstance(v, GLib.Variant):
                out[k] = v
            elif isinstance(v, bool):
                out[k] = GLib.Variant("b", v)
            elif isinstance(v, int):
                out[k] = GLib.Variant("t", v)
            else:
                out[k] = GLib.Variant("s", str(v))
        return out

    # -- state -----------------------------------------------------------------
    def managed_objects(self) -> dict:
        r = self._call(UD_PATH, OM, "GetManagedObjects", reply="(a{oa{sa{sv}}})", timeout_ms=10_000)
        return r[0] if r else {}

    def _boot_entries(self) -> list[dict]:
        """Firmware boot entries (efivarfs, read-only), cached for 30 s."""
        if self.efi_boot is None:
            return []
        t, entries = self._boot
        if time.monotonic() - t > 30:
            try:
                entries = list(self.efi_boot() or [])
            except Exception:  # noqa: BLE001
                log.debug("efi boot entries unreadable", exc_info=True)
                entries = []
            self._boot = (time.monotonic(), entries)
        return entries

    def snapshot(self, refresh: bool = False) -> dict:
        with self._lock:
            if not refresh and self._snap is not None and time.monotonic() - self._snap_t < 1.0:
                return self._snap
        try:
            objs = self.managed_objects()
        except ApiError as e:
            if e.code in ("unavailable", "timeout", "failed", "not_found"):
                snap = {"supported": False, "error": e.message, "disks": [], "jobs": [],
                        "filesystems": filesystems(None, self.which), "encryption": {"supported": False, "types": []}}
                with self._lock:
                    self._snap, self._snap_t = snap, time.monotonic()
                return snap
            raise
        snap = parse_managed_objects(objs, statvfs=self.statvfs, shell_paths=self.shell_paths,
                                     swap_files=self.swap_files(), slaves_of=self.slaves_of, which=self.which,
                                     boot_entries=self._boot_entries())
        snap["readOnly"] = self.readonly
        with self._lock:
            self._snap, self._snap_t = snap, time.monotonic()
        return snap

    def list(self, refresh: bool = False) -> dict:
        try:
            return self.snapshot(refresh=refresh)
        except Exception as e:  # noqa: BLE001 - no Gio, no system bus …
            log.info("Schijven niet beschikbaar (disks unavailable): %s", e)
            return {"supported": False, "error": _tr("unavailable"), "disks": [], "jobs": [],
                    "filesystems": [], "encryption": {"supported": False, "types": []}}

    def refresh(self, emit: bool = True) -> dict:
        snap = self.list(refresh=True)
        sig = json.dumps(snap, sort_keys=True, default=str)
        with self._lock:
            changed = sig != self._last_sig
            self._last_sig = sig
        if emit and changed:
            self.emit("disks.changed", snap)
        return snap

    def _fresh(self) -> dict:
        snap = self.snapshot(refresh=True)
        if not snap.get("supported"):
            raise ApiError("unavailable", snap.get("error") or _tr("unavailable"))
        return snap

    @staticmethod
    def _check_id(v: Any) -> str:
        if not isinstance(v, str) or not _ID_RE.match(v):
            raise ApiError("bad_args", _tr("bad_args"))
        return v

    def find_volume(self, snap: dict, vid: Any) -> tuple[dict, dict]:
        vid = self._check_id(vid)
        for d in snap["disks"]:
            for v in d["volumes"]:
                if v["id"] == vid or (v.get("encrypted") or {}).get("cleartext") == vid:
                    return d, v
        raise ApiError("not_found", _tr("gone_volume"))

    def find_disk(self, snap: dict, did: Any) -> dict:
        """By the disk's id (its whole block); a Drive path is accepted when exactly one disk belongs to it."""
        did = self._check_id(did)
        for d in snap["disks"]:
            if d["id"] == did or d["block"] == did:
                return d
        on_drive = [d for d in snap["disks"] if d.get("drive") == did]
        if len(on_drive) == 1:
            return on_drive[0]
        raise ApiError("not_found", _tr("gone_disk"))

    # -- what belongs together -------------------------------------------------
    @staticmethod
    def _siblings(snap: dict, d: dict) -> list[dict]:
        """Every disk on the same Drive (NVMe namespaces …): Drive.PowerOff / Eject act on all of them."""
        if not d.get("drive"):
            return [d]
        return [x for x in snap["disks"] if x.get("drive") == d["drive"]] or [d]

    def _keys(self, snap: dict, d: dict) -> tuple[str, ...]:
        return tuple({x.get("drive") or x["block"] for x in self._siblings(snap, d)})

    @staticmethod
    def _vol_paths(v: dict) -> set[str]:
        enc = v.get("encrypted") or {}
        return {p for p in (v["id"], enc.get("cleartext")) if p}

    def _disk_paths(self, disks: list[dict]) -> set[str]:
        out: set[str] = set()
        for d in disks:
            out.update(p for p in (d["block"], d.get("drive")) if p)
            for v in d["volumes"]:
                out |= self._vol_paths(v)
        return out

    @staticmethod
    def _jobs_on(snap: dict, paths: set[str]) -> list[dict]:
        return [j for j in snap.get("jobs") or [] if paths & set(j.get("objects") or [])]

    def _no_jobs(self, snap: dict, paths: set[str], name: str) -> None:
        if self._jobs_on(snap, paths):
            raise ApiError("busy_job", _tr("busy_job", name=name))

    @staticmethod
    def _expect(obj: dict, expect: Any) -> None:
        """Destructive calls name what the user saw: {device, size}. A mismatch means the object path now points at
        another device (a stick replaced, a partition recreated)."""
        if not isinstance(expect, dict) or "size" not in expect or "device" not in expect:
            raise ApiError("bad_args", _tr("expect"))
        try:
            same = int(expect["size"]) == int(obj["size"]) and str(expect["device"]) == str(obj["device"])
        except (TypeError, ValueError):
            same = False
        if same and expect.get("uuid") and obj.get("uuid") and expect["uuid"] != obj["uuid"]:
            same = False
        if not same:
            raise ApiError("changed", _tr("changed"))

    @staticmethod
    def _refuse(reasons: list[str], op: str, disk: bool = False, names: list[str] | tuple = ()) -> None:
        if not reasons:
            return
        hard = hard_reasons(reasons)
        nm = ", ".join(f"‘{n}’" for n in names) or "?"
        if disk:
            why = _tr("r_disk") if hard else _tr("r_disk_boot", names=nm)
        else:
            why = _tr(f"r_{(hard or reasons)[0]}", names=nm)
        raise ApiError("protected", f"{why} {_tr('no_' + op)}", {"reasons": list(reasons)})

    def _operation(self, keys) -> _Op:
        """One operation per drive at a time (a second one is refused, never queued behind a long format)."""
        return _Op(self, keys)

    # -- checks: the same for the action and its --dry-run plan -----------------
    def _volume_checks(self, snap: dict, op: str, args: dict) -> tuple[dict, dict]:
        d, v = self.find_volume(snap, args.get("id"))
        paths = self._vol_paths(v)
        if op == "unmount":
            self._refuse(hard_reasons(v["protected"]), "unmount")
            if not v["mounted"]:
                raise ApiError("not_mounted", _tr("not_mounted"))
            self._no_jobs(snap, paths, v["name"])
        elif op in ("mount", "open"):
            if not v["mounted"]:
                self._mount_target(v)
                self._no_jobs(snap, paths, v["name"])
        elif op == "setLabel":
            self._refuse(v["protected"], "label", names=v["bootEntries"])
            self._label_target(v, args.get("label"))
            self._no_jobs(snap, paths, v["name"])
        elif op == "format":
            self._refuse(v["protected"], "format", names=v["bootEntries"])
            self._expect(v, args.get("expect"))
            self._formattable(v)
            self._format_args(snap, args, d, partition=v["kind"] == "partition")
            self._no_jobs(snap, paths, v["name"])
        elif op == "deletePartition":
            self._refuse(v["protected"], "delete", names=v["bootEntries"])
            self._expect(v, args.get("expect"))
            self._delete_target(d, v)
            self._no_jobs(snap, paths, v["name"])
        elif op == "unlock":
            if not v.get("encrypted"):
                raise ApiError("not_encrypted", _tr("not_encrypted"))
            if not isinstance(args.get("passphrase"), str) or not args.get("passphrase"):
                raise ApiError("bad_args", _tr("passphrase_empty"))
        elif op == "lock":
            if not v.get("encrypted"):
                raise ApiError("not_encrypted", _tr("not_encrypted"))
            self._refuse(hard_reasons(v["protected"]), "lock")
            self._no_jobs(snap, paths, v["name"])
        return d, v

    def _disk_checks(self, snap: dict, op: str, args: dict) -> dict:
        d = self.find_disk(snap, args.get("disk") or args.get("id"))
        if op in ("eject", "powerOff"):
            sib = self._siblings(snap, d)
            self._refuse(hard_reasons([r for x in sib for r in x["protected"]]), "eject", disk=True)
            if d["kind"] == "loop" or not d.get("drive"):
                raise ApiError("not_removable", _tr("not_removable"))
            self._no_jobs(snap, self._disk_paths(sib), d["model"])
        elif op == "initialize":
            self._refuse(d["protected"], "initialize", disk=True, names=d["bootEntries"])
            self._expect(d, args.get("expect"))
            if args.get("table", "gpt") not in ("gpt", "dos"):
                raise ApiError("bad_args", _tr("bad_table"))
            self._no_jobs(snap, self._disk_paths([d]), d["name"])
        elif op == "createPartition":
            if args.get("expect") is not None:
                self._expect(d, args.get("expect"))
            self._create_args(snap, d, args)
            self._no_jobs(snap, self._disk_paths([d]), d["name"])
        return d

    # -- plans (also answer in --dry-run: the same refusals, nothing changed) ---------
    def plan(self, op: str, args: dict) -> dict:
        snap = self._fresh()
        if op in ("mount", "unmount", "open", "setLabel", "format", "deletePartition", "unlock", "lock"):
            _d, v = self._volume_checks(snap, op, args)
            return {"id": v["id"], "device": v["device"]}
        d = self._disk_checks(snap, op, args)
        return {"disk": d["id"], "device": d["device"]}

    # -- argument checks ---------------------------------------------------------
    def _mount_target(self, v: dict) -> str:
        enc = v.get("encrypted")
        if enc and not enc["unlocked"]:
            raise ApiError("locked", _tr("locked"))
        target = enc["cleartext"] if enc else v["id"]
        if not v["fsType"] or v["fsType"] in ("swap", "crypto_LUKS", "LVM2_member", "linux_raid_member") or \
                (enc and not enc.get("inner")):
            raise ApiError("no_fs", _tr("no_fs"))
        return target

    def _label_target(self, v: dict, label: Any) -> tuple[str, str]:
        enc = v.get("encrypted")
        if enc and not enc["unlocked"]:
            raise ApiError("locked", _tr("locked"))
        if v["fsType"] not in LABEL_RULES:
            raise ApiError("no_label_fs", _tr("no_label_fs"))
        if v.get("fstabByLabel"):
            raise ApiError("in_use", _tr("fstab_label"))
        return (enc["cleartext"] if enc else v["id"]), check_label(v["fsType"], label if label is not None else "")

    def _fs_ok(self, snap: dict, fs: Any) -> str:
        if not isinstance(fs, str) or fs not in FILESYSTEMS:
            raise ApiError("bad_args", _tr("fs_unknown"))
        info = next((f for f in snap.get("filesystems") or [] if f["id"] == fs), None)
        if not info or not info["available"]:
            raise ApiError("fs_missing", _tr("fs_missing", fs=FILESYSTEMS[fs][0]))
        return fs

    def _passphrase(self, snap: dict, args: dict) -> str | None:
        enc = args.get("encrypt")
        if not enc:
            return None
        pw = enc.get("passphrase") if isinstance(enc, dict) else None
        if not isinstance(pw, str) or len(pw) < 8:
            raise ApiError("bad_args", _tr("passphrase"))
        if not snap.get("encryption", {}).get("supported"):
            raise ApiError("not_supported", _tr("failed", detail="LUKS"))
        return pw

    @staticmethod
    def _formattable(v: dict) -> None:
        if v.get("role") in ("system", "container"):
            raise ApiError("system_partition", _tr("system_part", type=v.get("partTypeName") or v.get("name")))

    def _format_args(self, snap: dict, args: dict, d: dict, partition: bool) -> dict:
        fs = self._fs_ok(snap, args.get("fsType"))
        label = check_label(fs, args.get("label") or "")
        pw = self._passphrase(snap, args)
        opts: dict[str, Any] = {"tear-down": True}
        if label:
            opts["label"] = label
        if partition:
            opts["update-partition-type"] = True
        if FILESYSTEMS[fs][4]:
            opts["take-ownership"] = True
        if args.get("quick", True) is False:
            opts["erase"] = "zero"
        if pw is not None:
            opts["encrypt.passphrase"] = pw
            opts["encrypt.type"] = snap.get("encryption", {}).get("default") or "luks2"
        return {"fs": fs, "opts": opts}

    def _delete_target(self, d: dict, v: dict) -> None:
        if v["kind"] != "partition":
            raise ApiError("not_partition", _tr("not_partition"))
        if v.get("role") == "system":
            raise ApiError("system_partition", _tr("system_part", type=v.get("partTypeName") or v.get("name")))
        if v.get("partTypeKey") == "ldm_data":
            raise ApiError("dynamic_partition", _tr("dynamic_part"))
        if v["isContainer"] and any(x["isContained"] for x in d["volumes"]):
            raise ApiError("not_empty", _tr("has_logical"))

    def _create_args(self, snap: dict, d: dict, args: dict) -> dict:
        if not d["partitionTable"]:
            raise ApiError("no_table", _tr("no_table"))
        if d.get("readOnly"):
            raise ApiError("read_only", _tr("failed", detail="read-only"))
        try:
            offset = int(args.get("offset"))
            size = int(args.get("size") or 0)
        except (TypeError, ValueError):
            raise ApiError("bad_args", _tr("bad_args")) from None
        region = next((r for r in d["free"] if r["offset"] <= offset < r["offset"] + r["size"]), None)
        if region is None:
            raise ApiError("no_free", _tr("no_free"))
        logical = bool(region.get("logical"))
        if not region.get("canCreate", True):
            raise ApiError("table_full", _tr("table_full")) if d["tableFull"] else ApiError("no_free", _tr("no_free"))
        room = region["offset"] + region["size"] - offset
        if size <= 0 or size > room:
            size = room
        if size < MIN_VOLUME:
            raise ApiError("too_small", _tr("too_small"))
        if size < room and room - size < MiB:
            size = room                      # no 300 KB leftovers
        fmt = self._format_args(snap, args, d, partition=False)
        fmt["opts"].pop("update-partition-type", None)
        fs = fmt["fs"]
        ptype = GPT_FOR_FS[fs] if d["partitionTable"] == "gpt" else DOS_FOR_FS[fs]
        popts: dict[str, Any] = {}
        if d["partitionTable"] == "dos":
            popts["partition-type"] = "logical" if logical else "primary"
        return {"offset": offset, "size": size, "type": ptype, "opts": popts, "fs": fs, "fopts": fmt["opts"],
                "logical": logical}

    # -- small helpers -------------------------------------------------------------
    def _unmount_path(self, path: str, name: str) -> None:
        self._call(path, I_FS, "Unmount", "(a{sv})", (self._opts({}),), name=name)

    def _teardown_volume(self, v: dict) -> bool:
        """Unmount (and lock) a volume before it is formatted / deleted / the disk removed. → was mounted."""
        was = False
        enc = v.get("encrypted")
        if enc and enc["unlocked"]:
            if v["mounted"]:
                self._unmount_path(enc["cleartext"], v["name"])
                was = True
            self._call(v["id"], I_CRYPT, "Lock", "(a{sv})", (self._opts({}),), name=v["name"])
        elif v["mounted"]:
            self._unmount_path(v["id"], v["name"])
            was = True
        return was

    def _mount_new(self, path: str, name: str) -> tuple[str | None, str | None]:
        """Mount a freshly made (or relabelled) file system (non-fatal) → (mount point, error message)."""
        try:
            snap = self.snapshot(refresh=True)
            _d, v = self.find_volume(snap, path)
            target = self._mount_target(v)
            r = self._call(target, I_FS, "Mount", "(a{sv})", (self._opts({}),), reply="(s)", name=name)
            return (r[0] if r else None), None
        except ApiError as e:
            return None, e.message

    def _done(self) -> None:
        try:
            self.refresh(emit=True)
        except Exception:  # noqa: BLE001
            log.debug("refresh after action failed", exc_info=True)

    # -- actions ---------------------------------------------------------------------
    def mount(self, vid: Any) -> dict:
        snap = self._fresh()
        _d, v = self.find_volume(snap, vid)
        if v["mounted"]:
            return {"ok": True, "mountPoint": v["mountPoint"], "already": True}
        self._volume_checks(snap, "mount", {"id": vid})
        target = self._mount_target(v)
        log.info("Volume koppelen (mount) %s", v["device"])
        r = self._call(target, I_FS, "Mount", "(a{sv})", (self._opts({}),), reply="(s)", name=v["name"])
        self._done()
        return {"ok": True, "mountPoint": r[0] if r else None}

    def unmount(self, vid: Any) -> dict:
        snap = self._fresh()
        d, v = self.find_volume(snap, vid)
        self._refuse(hard_reasons(v["protected"]), "unmount")
        if not v["mounted"]:
            return {"ok": True, "already": True}
        with self._operation(self._keys(snap, d)):
            self._volume_checks(snap, "unmount", {"id": vid})
            enc = v.get("encrypted")
            log.info("Volume ontkoppelen (unmount) %s", v["device"])
            self._unmount_path(enc["cleartext"] if enc and enc["unlocked"] else v["id"], v["name"])
        self._done()
        return {"ok": True}

    def open(self, vid: Any, dry: bool = False) -> dict:
        snap = self._fresh()
        _d, v = self.find_volume(snap, vid)
        path = v["mountPoint"]
        mounted = False
        if not path:
            self._volume_checks(snap, "open", {"id": vid})
            if dry:
                return {"ok": True, "dryRun": True, "mounted": False, "path": None}
            path = self.mount(vid).get("mountPoint")
            mounted = True
        if path and self.open_app is not None:
            self.open_app("explorer", {"path": path})
        return {"ok": True, "path": path, "mounted": mounted}

    def set_label(self, vid: Any, label: Any) -> dict:
        """Label wijzigen. NTFS, exFAT, FAT, Btrfs, XFS … can't be relabelled while mounted (ntfslabel refuses):
        such a volume is unmounted, relabelled and mounted again (busy → nothing changed)."""
        snap = self._fresh()
        d, v = self.find_volume(snap, vid)
        self._refuse(v["protected"], "label", names=v["bootEntries"])
        target, label = self._label_target(v, label)
        mp, merr = None, None
        with self._operation(self._keys(snap, d)):
            self._volume_checks(snap, "setLabel", {"id": vid, "label": label})
            remount = bool(v["mounted"] and v["fsType"] in OFFLINE_LABEL)
            log.info("Volumenaam wijzigen (set label) %s%s", v["device"], " (ontkoppeld)" if remount else "")
            if remount:
                self._unmount_path(target, v["name"])
            try:
                self._call(target, I_FS, "SetLabel", "(sa{sv})", (label, self._opts({})), name=v["name"])
            finally:
                if remount:
                    mp, merr = self._mount_new(v["id"], v["name"])
        self._done()
        return {"ok": True, "label": label, "remounted": bool(v["mounted"] and v["fsType"] in OFFLINE_LABEL),
                "mountPoint": mp, "mountError": merr}

    def format(self, args: dict) -> dict:
        snap = self._fresh()
        d, v = self.find_volume(snap, args.get("id"))
        with self._operation(self._keys(snap, d)) as op:
            snap = self._fresh()
            d, v = self._volume_checks(snap, "format", args)
            plan = self._format_args(snap, args, d, partition=v["kind"] == "partition")
            log.info("Volume formatteren (format) %s als %s%s", v["device"], plan["fs"],
                     " (versleuteld)" if "encrypt.passphrase" in plan["opts"] else "")
            was = self._teardown_volume(v)
            self._job_call(op, self._vol_paths(v), v["id"], I_BLOCK, "Format", "(sa{sv})",
                           (plan["fs"], self._opts(plan["opts"])), name=v["name"])
            mp, merr = (None, None)
            if was or args.get("mount"):
                mp, merr = self._mount_new(v["id"], v["name"])
        self._done()
        return {"ok": True, "id": v["id"], "mountPoint": mp, "mountError": merr}

    def delete_partition(self, args: dict) -> dict:
        snap = self._fresh()
        d, v = self.find_volume(snap, args.get("id"))
        with self._operation(self._keys(snap, d)) as op:
            snap = self._fresh()
            d, v = self._volume_checks(snap, "deletePartition", args)
            log.info("Partitie verwijderen (delete partition) %s", v["device"])
            self._teardown_volume(v)
            self._job_call(op, self._vol_paths(v) | {d["block"]}, v["id"], I_PART, "Delete", "(a{sv})",
                           (self._opts({"tear-down": True}),), name=v["name"])
        self._done()
        return {"ok": True}

    def create_partition(self, args: dict) -> dict:
        snap = self._fresh()
        d = self.find_disk(snap, args.get("disk"))
        with self._operation(self._keys(snap, d)) as op:
            snap = self._fresh()
            d = self._disk_checks(snap, "createPartition", args)
            p = self._create_args(snap, d, args)
            log.info("Nieuw volume (create partition) op %s: %d bytes %s%s", d["device"], p["size"], p["fs"],
                     " (logisch station)" if p["logical"] else "")
            r = self._job_call(op, self._disk_paths([d]), d["block"], I_TABLE, "CreatePartitionAndFormat",
                               "(ttssa{sv}sa{sv})", (p["offset"], p["size"], p["type"], "", self._opts(p["opts"]),
                                                     p["fs"], self._opts(p["fopts"])), reply="(o)", name=d["name"])
            new = str(r[0]) if r else None
            mp, merr = (None, None)
            if new and args.get("mount", True):
                mp, merr = self._mount_new(new, d["name"])
        self._done()
        return {"ok": True, "id": new, "mountPoint": mp, "mountError": merr}

    def initialize(self, args: dict) -> dict:
        snap = self._fresh()
        d = self.find_disk(snap, args.get("disk"))
        with self._operation(self._keys(snap, d)) as op:
            snap = self._fresh()
            d = self._disk_checks(snap, "initialize", args)
            table = args.get("table", "gpt")
            log.info("Schijf initialiseren (initialise) %s als %s", d["device"], table)
            for v in d["volumes"]:
                self._teardown_volume(v)
            self._job_call(op, self._disk_paths([d]), d["block"], I_BLOCK, "Format", "(sa{sv})",
                           (table, self._opts({"tear-down": True})), name=d["name"])
        self._done()
        return {"ok": True}

    def eject(self, did: Any) -> dict:
        snap = self._fresh()
        d = self.find_disk(snap, did)
        with self._operation(self._keys(snap, d)):
            snap = self._fresh()
            d = self._disk_checks(snap, "eject", {"disk": d["id"]})
            log.info("Schijf uitwerpen (eject) %s", d["device"])
            for x in self._siblings(snap, d):
                for v in x["volumes"]:
                    self._teardown_volume(v)
            self._call(d["drive"], I_DRIVE, "Eject", "(a{sv})", (self._opts({}),), name=d["model"])
        self._done()
        return {"ok": True}

    def power_off(self, did: Any) -> dict:
        """"Veilig verwijderen": unmount everything on the drive, lock LUKS, then power it off (or eject it).
        Refused while a UDisks job (format, erase …) runs on any of its volumes."""
        snap = self._fresh()
        d = self.find_disk(snap, did)
        with self._operation(self._keys(snap, d)):
            snap = self._fresh()
            d = self._disk_checks(snap, "powerOff", {"disk": d["id"]})
            log.info("Veilig verwijderen (power off) %s", d["device"])
            for x in self._siblings(snap, d):
                for v in x["volumes"]:
                    self._teardown_volume(v)
            how = "unmounted"
            if d["canPowerOff"]:
                self._call(d["drive"], I_DRIVE, "PowerOff", "(a{sv})", (self._opts({}),), name=d["model"])
                how = "poweroff"
            elif d["ejectable"]:
                self._call(d["drive"], I_DRIVE, "Eject", "(a{sv})", (self._opts({}),), name=d["model"])
                how = "eject"
        self._done()
        return {"ok": True, "how": how}

    def unlock(self, vid: Any, passphrase: Any, mount: bool = True) -> dict:
        snap = self._fresh()
        _d, v = self.find_volume(snap, vid)
        enc = v.get("encrypted")
        if not enc:
            raise ApiError("not_encrypted", _tr("not_encrypted"))
        if enc["unlocked"]:
            return {"ok": True, "cleartext": enc["cleartext"], "already": True, "mountPoint": v["mountPoint"]}
        if not isinstance(passphrase, str) or not passphrase:
            raise ApiError("bad_args", _tr("passphrase_empty"))
        log.info("Volume ontgrendelen (unlock) %s", v["device"])
        r = self._call(v["id"], I_CRYPT, "Unlock", "(sa{sv})", (passphrase, self._opts({})), reply="(o)",
                       name=v["name"])
        clear = str(r[0]) if r else None
        mp, merr = (None, None)
        if mount and clear:
            mp, merr = self._mount_new(v["id"], v["name"])
        self._done()
        return {"ok": True, "cleartext": clear, "mountPoint": mp, "mountError": merr}

    def lock(self, vid: Any) -> dict:
        snap = self._fresh()
        d, v = self.find_volume(snap, vid)
        enc = v.get("encrypted")
        if not enc:
            raise ApiError("not_encrypted", _tr("not_encrypted"))
        self._refuse(hard_reasons(v["protected"]), "lock")
        if not enc["unlocked"]:
            return {"ok": True, "already": True}
        with self._operation(self._keys(snap, d)):
            self._volume_checks(snap, "lock", {"id": vid})
            log.info("Volume vergrendelen (lock) %s", v["device"])
            self._teardown_volume(v)
        self._done()
        return {"ok": True}

    def smart(self, did: Any, refresh: bool = False) -> dict:
        snap = self._fresh()
        d = self.find_disk(snap, did)
        out = dict(d["smart"])
        if not out.get("supported") or not d.get("drive"):
            out["message"] = _tr("no_smart")
            return out
        iface = I_NVME if out.get("kind") == "nvme" else I_ATA
        if refresh:
            self._call(d["drive"], iface, "SmartUpdate", "(a{sv})", (self._opts({}),), timeout_ms=CALL_TIMEOUT_MS,
                       name=d["model"])
            snap = self.snapshot(refresh=True)
            d = self.find_disk(snap, d["id"])
            out = dict(d["smart"])
        try:
            if iface == I_NVME:
                r = self._call(d["drive"], iface, "SmartGetAttributes", "(a{sv})", (self._opts({}),), reply="(a{sv})",
                               timeout_ms=CALL_TIMEOUT_MS, name=d["model"])
                out.update(nvme_health(r[0] if r else {}))
            else:
                r = self._call(d["drive"], iface, "SmartGetAttributes", "(a{sv})", (self._opts({}),),
                               reply="(a(ysqiiixia{sv}))", timeout_ms=CALL_TIMEOUT_MS, name=d["model"])
                out["attributes"] = ata_attributes(r[0] if r else [])
        except ApiError as e:
            if e.code == "readonly":
                out["details"] = False
            else:
                out["detailsError"] = e.message
        return out

    # -- watching ----------------------------------------------------------------------
    def start_watch(self) -> bool:
        if self._watch is not None:
            return True
        try:
            self._watch = _Watcher(self)
            self._watch.start()
            return True
        except Exception as e:  # noqa: BLE001
            self._watch = None
            log.info("Schijven: wijzigingen niet te volgen (cannot watch UDisks): %s", e)
            return False

    def stop(self) -> None:
        w, self._watch = self._watch, None
        if w is not None:
            w.stop()


def nvme_health(a: dict) -> dict:
    def n(k):
        v = a.get(k)
        return int(v) if isinstance(v, (int, float)) and not isinstance(v, bool) else None
    used = n("percent_used")
    spare, thresh = n("avail_spare"), n("spare_thresh")
    out = {"percentUsed": used, "lifeRemaining": max(0, 100 - used) if used is not None else None,
           "availableSpare": spare, "spareThreshold": thresh, "dataRead": n("total_data_read"),
           "dataWritten": n("total_data_written"), "powerCycles": n("power_cycles"),
           "unsafeShutdowns": n("unsafe_shutdowns"), "mediaErrors": n("media_errors"),
           "errorLogEntries": n("num_err_log_entries"), "warningTempMinutes": n("warning_temp_time"),
           "criticalTempMinutes": n("critical_temp_time"), "details": True}
    temps = a.get("temp_sensors")
    if isinstance(temps, (list, tuple)):
        out["sensors"] = [t for t in (_kelvin(x) for x in temps) if t is not None]
    return out


def ata_attributes(rows: list) -> list[dict]:
    out = []
    for row in rows or []:
        try:
            aid, name, flags, value, worst, thresh, pretty, unit, _exp = row
        except (TypeError, ValueError):
            continue
        valid = value >= 0
        out.append({"id": int(aid), "name": str(name), "prefail": bool(int(flags) & 1),
                    "value": value if valid else None, "worst": worst if worst >= 0 else None,
                    "threshold": thresh if thresh >= 0 else None, "pretty": int(pretty), "unit": int(unit),
                    "failing": bool(valid and thresh > 0 and value <= thresh),
                    "failedInPast": bool(worst >= 0 and thresh > 0 and worst <= thresh)})
    return out


class _Watcher:
    """Follows UDisks signals in an own thread + GLib main context; debounced refresh → ``disks.changed``."""

    def __init__(self, svc: DisksService):
        self.svc = svc
        self._stop = threading.Event()
        self._mctx = None
        self._pending = False
        self._subs: list[int] = []
        self._thread: threading.Thread | None = None
        self._ready = threading.Event()
        self._error: Exception | None = None

    def start(self) -> None:
        self._thread = threading.Thread(target=self._run, name="disks-watch", daemon=True)
        self._thread.start()
        self._ready.wait(5)
        if self._error is not None:
            raise self._error

    def _run(self) -> None:
        Gio, GLib = _gi()
        self._mctx = GLib.MainContext.new()
        self._mctx.push_thread_default()
        try:
            try:
                bus = self.svc.bus()
                f = Gio.DBusSignalFlags.NONE
                self._subs.append(bus.signal_subscribe(UD, OM, None, UD_PATH, None, f, self._on_signal))
                self._subs.append(bus.signal_subscribe(UD, "org.freedesktop.DBus.Properties", "PropertiesChanged",
                                                       None, None, f, self._on_signal))
                self._subs.append(bus.signal_subscribe("org.freedesktop.DBus", "org.freedesktop.DBus",
                                                       "NameOwnerChanged", "/org/freedesktop/DBus", UD, f,
                                                       self._on_signal))
                self.svc.refresh(emit=False)       # the baseline: later signals compare against it
            except Exception as e:  # noqa: BLE001
                self._error = e
                return
            finally:
                self._ready.set()
            while not self._stop.is_set():
                self._mctx.iteration(True)
            for sid in self._subs:
                try:
                    bus.signal_unsubscribe(sid)
                except Exception:  # noqa: BLE001
                    pass
        finally:
            self._mctx.pop_thread_default()

    def _on_signal(self, *_a) -> None:
        if self._pending or self._stop.is_set():
            return
        self._pending = True
        _, GLib = _gi()
        src = GLib.timeout_source_new(int(DEBOUNCE_S * 1000))
        src.set_callback(self._flush)
        src.attach(self._mctx)

    def _flush(self, *_a) -> bool:
        self._pending = False
        try:
            self.svc.refresh(emit=True)
        except Exception:  # noqa: BLE001
            log.debug("disks watch refresh failed", exc_info=True)
        return False

    def stop(self) -> None:
        self._stop.set()
        if self._mctx is not None:
            self._mctx.wakeup()
        if self._thread is not None:
            self._thread.join(2)


# ---------------------------------------------------------------------------
# registration
# ---------------------------------------------------------------------------

_state: dict[str, Any] = {}


def register(api, ctx, service: DisksService | None = None):
    global _lang

    def lang() -> str:
        try:
            return "en" if ctx.settings.get("language", "nl") == "en" else "nl"
        except Exception:  # noqa: BLE001
            return "nl"

    _lang = lang

    def open_app(app, params):
        return ctx.open_app(app, params)

    svc = service or DisksService(emit=api.emit, open_app=open_app)
    if service is not None and service.open_app is None:
        service.open_app = open_app
    _state["svc"] = svc

    def plan(op):
        return lambda args: svc.plan(op, args)

    @api.method("disks.list")
    def _list(args):
        """{} → {supported, disks:[Disk], jobs, filesystems, encryption, version, readOnly}"""
        return svc.list(refresh=bool(args.get("refresh")))

    @api.method("disks.mount", dangerous=True, dry_result=plan("mount"))
    def _mount(args):
        """{id} → {ok, mountPoint}"""
        return svc.mount(args.get("id"))

    @api.method("disks.unmount", dangerous=True, dry_result=plan("unmount"))
    def _unmount(args):
        """{id} → {ok}"""
        return svc.unmount(args.get("id"))

    @api.method("disks.open")
    def _open(args):
        """{id} → {ok, path, mounted}: Verkenner at the mount point (mounts first; in --dry-run only plans that)"""
        return svc.open(args.get("id"), dry=dry_for(ctx) or test_guard_blocks("disks.mount"))

    @api.method("disks.setLabel", dangerous=True, dry_result=plan("setLabel"))
    def _set_label(args):
        """{id, label} → {ok, label}"""
        return svc.set_label(args.get("id"), args.get("label"))

    @api.method("disks.format", dangerous=True, dry_result=plan("format"))
    def _format(args):
        """{id, fsType, label, quick=true, encrypt?:{passphrase}, expect:{device, size}} → {ok, id, mountPoint}"""
        return svc.format(args)

    @api.method("disks.deletePartition", dangerous=True, dry_result=plan("deletePartition"))
    def _delete(args):
        """{id, expect:{device, size}} → {ok}"""
        return svc.delete_partition(args)

    @api.method("disks.createPartition", dangerous=True, dry_result=plan("createPartition"))
    def _create(args):
        """{disk, offset, size, fsType, label, encrypt?, expect?} → {ok, id, mountPoint}"""
        return svc.create_partition(args)

    @api.method("disks.initialize", dangerous=True, dry_result=plan("initialize"))
    def _initialize(args):
        """{disk, table:'gpt'|'dos', expect} → {ok}"""
        return svc.initialize(args)

    @api.method("disks.eject", dangerous=True, dry_result=plan("eject"))
    def _eject(args):
        """{disk} → {ok}"""
        return svc.eject(args.get("disk") or args.get("id"))

    @api.method("disks.powerOff", dangerous=True, dry_result=plan("powerOff"))
    def _power_off(args):
        """{disk} → {ok, how:'poweroff'|'eject'|'unmounted'} ("Veilig verwijderen")"""
        return svc.power_off(args.get("disk") or args.get("id"))

    @api.method("disks.unlock", dangerous=True, dry_result=plan("unlock"))
    def _unlock(args):
        """{id, passphrase, mount=true} → {ok, cleartext, mountPoint}"""
        return svc.unlock(args.get("id"), args.get("passphrase"), args.get("mount", True) is not False)

    @api.method("disks.lock", dangerous=True, dry_result=plan("lock"))
    def _lock(args):
        """{id} → {ok}"""
        return svc.lock(args.get("id"))

    @api.method("disks.smart")
    def _smart(args):
        """{disk, refresh?} → health summary (+ NVMe health log / ATA attributes)"""
        refresh = bool(args.get("refresh")) and not dry_for(ctx)
        return svc.smart(args.get("disk") or args.get("id"), refresh=refresh)


def start(api, ctx):
    svc: DisksService | None = _state.get("svc")
    if svc is not None:
        svc.start_watch()


def stop(api, ctx):
    svc: DisksService | None = _state.get("svc")
    if svc is not None:
        svc.stop()
