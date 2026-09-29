package releaseverification

import (
	"bytes"
	"debug/pe"
	"encoding/binary"
	"errors"
	"fmt"
	"io"
	"strings"

	"aster.local/team/operations/backend/internal/domain"
)

type platformPolicy struct {
	target        domain.ReleaseTarget
	runtime       string
	sbomRuntime   string
	rustTarget    string
	binaries      map[string]bool
	required      map[string]bool
	topLevels     string
	inspectBinary func(io.ReaderAt) error
}

func policyFor(target domain.ReleaseTarget) (platformPolicy, error) {
	if !target.Supported() {
		return platformPolicy{}, errors.New("unsupported release target")
	}
	common := map[string]bool{"README.md": false, "SBOM.cdx.json": false, "VERSION": false}
	policy := platformPolicy{target: target, required: common}
	switch target.Platform {
	case "linux":
		policy.runtime = "musl-static"
		policy.sbomRuntime = "musl-static"
		policy.rustTarget = "x86_64-unknown-linux-musl"
		policy.binaries = map[string]bool{"bin/aster-team-cli": true, "bin/aster-control": true, "bin/aster-runner": true}
		policy.required["init.sh"] = true
		policy.required["libexec/install.sh"] = true
		policy.required["libexec/restore-backup.sh"] = true
		policy.topLevels = "README.md SBOM.cdx.json THIRD_PARTY_LICENSES VERSION admin bin client-tools init.sh libexec licenses member systemd"
		policy.inspectBinary = verifyStaticLinuxAMD64ELF
	case "windows":
		policy.runtime = "msvc"
		policy.sbomRuntime = "msvc-static"
		policy.rustTarget = "x86_64-pc-windows-msvc"
		policy.binaries = map[string]bool{"bin/aster-team-cli.exe": true, "bin/aster-control.exe": true, "bin/aster-runner.exe": true, "bin/caddy.exe": true, "client-tools/asterctl/windows-x86_64/asterctl.exe": false}
		policy.required["init.ps1"] = false
		policy.required["libexec/install.ps1"] = false
		policy.required["libexec/restore-backup.ps1"] = false
		policy.required["windows/service-launch.ps1"] = false
		policy.topLevels = "README.md SBOM.cdx.json THIRD_PARTY_LICENSES VERSION admin bin client-tools init.ps1 libexec licenses member windows"
		policy.inspectBinary = verifyStaticWindowsAMD64PE
	}
	for name, executable := range policy.binaries {
		policy.required[name] = executable
	}
	return policy, nil
}

func verifyStaticWindowsAMD64PE(source io.ReaderAt) error {
	file, err := pe.NewFile(source)
	if err != nil {
		return errors.New("binary is not a valid PE")
	}
	defer file.Close()
	header, ok := file.OptionalHeader.(*pe.OptionalHeader64)
	if !ok || file.Machine != pe.IMAGE_FILE_MACHINE_AMD64 || file.Characteristics&pe.IMAGE_FILE_EXECUTABLE_IMAGE == 0 || file.Characteristics&pe.IMAGE_FILE_DLL != 0 || header.Magic != 0x20b {
		return errors.New("binary is not a windows/amd64 PE executable")
	}
	if len(file.Sections) == 0 || header.AddressOfEntryPoint == 0 || header.NumberOfRvaAndSizes < 14 || (header.Subsystem != 2 && header.Subsystem != 3) {
		return errors.New("PE executable headers are invalid")
	}
	// Delay imports are not emitted by the supported static-CRT build. Reject them
	// rather than overlooking another dependency table.
	if header.DataDirectory[13].VirtualAddress != 0 || header.DataDirectory[13].Size != 0 {
		return errors.New("binary contains delay-loaded dependencies")
	}
	readRVA := func(rva uint32, size uint32) ([]byte, error) {
		for _, section := range file.Sections {
			if rva >= section.VirtualAddress && uint64(rva-section.VirtualAddress)+uint64(size) <= uint64(section.Size) {
				contents := make([]byte, size)
				_, err := section.ReadAt(contents, int64(rva-section.VirtualAddress))
				return contents, err
			}
		}
		return nil, errors.New("PE import address is outside its sections")
	}
	imports := header.DataDirectory[1]
	if imports.VirtualAddress == 0 && imports.Size == 0 {
		return nil
	}
	if imports.VirtualAddress == 0 || imports.Size < 20 || imports.Size > 1<<20 || uint64(imports.VirtualAddress)+uint64(imports.Size) > 1<<32 {
		return errors.New("PE import directory is invalid")
	}
	for offset := uint32(0); offset+20 <= imports.Size; offset += 20 {
		descriptor, err := readRVA(imports.VirtualAddress+offset, 20)
		if err != nil {
			return err
		}
		if bytes.Equal(descriptor, make([]byte, 20)) {
			return nil
		}
		nameRVA := binary.LittleEndian.Uint32(descriptor[12:16])
		var name []byte
		for index := uint32(0); index < 260; index++ {
			if uint64(nameRVA)+uint64(index) >= 1<<32 {
				return errors.New("PE import name overflows")
			}
			value, err := readRVA(nameRVA+index, 1)
			if err != nil {
				return err
			}
			if value[0] == 0 {
				break
			}
			name = append(name, value[0])
		}
		if len(name) == 0 || len(name) == 260 {
			return errors.New("PE import name is invalid")
		}
		library := strings.ToLower(string(name))
		if strings.HasPrefix(library, "vcruntime") || strings.HasPrefix(library, "msvcp") || strings.HasPrefix(library, "msvcr") || strings.HasPrefix(library, "ucrtbase") || strings.HasPrefix(library, "api-ms-win-crt-") {
			return fmt.Errorf("binary depends on a dynamic Microsoft C/C++ runtime: %s", library)
		}
	}
	return errors.New("PE import directory is unterminated")
}
