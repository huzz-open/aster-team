package releaseverification

import (
	"bytes"
	"context"
	"debug/pe"
	"encoding/binary"
	"testing"

	"aster.local/team/operations/backend/internal/config"
)

func fakeAMD64PE(dependency string) []byte {
	contents := make([]byte, 1024)
	copy(contents, "MZ")
	binary.LittleEndian.PutUint32(contents[60:], 128)
	var headers bytes.Buffer
	headers.WriteString("PE\x00\x00")
	binary.Write(&headers, binary.LittleEndian, pe.FileHeader{Machine: pe.IMAGE_FILE_MACHINE_AMD64, NumberOfSections: 1, SizeOfOptionalHeader: 240, Characteristics: pe.IMAGE_FILE_EXECUTABLE_IMAGE})
	optional := pe.OptionalHeader64{Magic: 0x20b, NumberOfRvaAndSizes: 16, SizeOfImage: 0x2000, SizeOfHeaders: 512, Subsystem: 3, AddressOfEntryPoint: 0x1000}
	optional.DataDirectory[1] = pe.DataDirectory{VirtualAddress: 0x1020, Size: 40}
	binary.Write(&headers, binary.LittleEndian, optional)
	section := pe.SectionHeader32{VirtualSize: 512, VirtualAddress: 0x1000, SizeOfRawData: 512, PointerToRawData: 512}
	copy(section.Name[:], ".text")
	binary.Write(&headers, binary.LittleEndian, section)
	copy(contents[128:], headers.Bytes())
	binary.LittleEndian.PutUint32(contents[512+32+12:], 0x1060)
	copy(contents[512+96:], dependency)
	return contents
}

func TestVerifierAcceptsWindowsSignedPackage(t *testing.T) {
	fixture := newReleaseFixture(t, false, releaseFixtureChanges{platform: "windows"})
	store := &memoryArtifactStore{}
	verifier, err := New(memoryArtifactSource{contents: fixture.zip}, store, config.ReleaseVerification{TrustedKeysJSON: fixture.keyring, MaxArtifactBytes: 64 << 20, MaxExpandedBytes: 128 << 20})
	if err != nil {
		t.Fatal(err)
	}
	verified, err := verifier.Verify(context.Background(), fixture.task, fixture.artifact)
	if err != nil {
		t.Fatal(err)
	}
	if verified.RuntimeLinkage != "msvc" || verified.SignatureKeyID != "release-test-01" || len(store.stored) == 0 {
		t.Fatalf("unexpected Windows result: %#v", verified)
	}
}

func TestWindowsVerifierRejectsDynamicCRTAndWrongBinaries(t *testing.T) {
	for _, dependency := range []string{"VCRUNTIME140.dll", "MSVCP140.dll", "ucrtbase.dll", "api-ms-win-crt-runtime-l1-1-0.dll"} {
		t.Run(dependency, func(t *testing.T) {
			fixture := newReleaseFixture(t, false, releaseFixtureChanges{platform: "windows", beforeSigning: func(files map[string]releaseFixtureFile) {
				files["bin/aster-control.exe"] = releaseFixtureFile{contents: fakeAMD64PE(dependency), executable: true}
			}})
			assertFixtureRejected(t, fixture, "RELEASE_ABI_INCOMPATIBLE", "runtime")
		})
	}
	for _, kind := range []string{"ELF", "arm64", "truncated"} {
		t.Run(kind, func(t *testing.T) {
			fixture := newReleaseFixture(t, false, releaseFixtureChanges{platform: "windows", beforeSigning: func(files map[string]releaseFixtureFile) {
				contents := fakeAMD64PE("KERNEL32.dll")
				switch kind {
				case "ELF":
					contents = fakeAMD64ELF()
				case "arm64":
					binary.LittleEndian.PutUint16(contents[132:], 0xaa64)
				case "truncated":
					contents = contents[:550]
				}
				files["bin/aster-runner.exe"] = releaseFixtureFile{contents: contents, executable: true}
			}})
			assertFixtureRejected(t, fixture, "RELEASE_ABI_INCOMPATIBLE", "")
		})
	}
}

func TestWindowsVerifierRejectsTamperingAndMissingLifecycleFiles(t *testing.T) {
	fixture := newReleaseFixture(t, false, releaseFixtureChanges{platform: "windows", afterSigning: func(files map[string]releaseFixtureFile) {
		files["init.ps1"] = releaseFixtureFile{contents: []byte("tampered")}
	}})
	assertFixtureRejected(t, fixture, "RELEASE_PACKAGE_POLICY_FAILED", "init.ps1")
	fixture = newReleaseFixture(t, false, releaseFixtureChanges{platform: "windows", beforeSigning: func(files map[string]releaseFixtureFile) { delete(files, "windows/service-launch.ps1") }})
	assertFixtureRejected(t, fixture, "RELEASE_PACKAGE_POLICY_FAILED", "service-launch.ps1")
	fixture = newReleaseFixture(t, true, releaseFixtureChanges{platform: "windows"})
	assertFixtureRejected(t, fixture, "RELEASE_SIGNATURE_INVALID", "")
}

func TestWindowsSBOMMustDescribeWindowsRuntime(t *testing.T) {
	fixture := newReleaseFixture(t, false, releaseFixtureChanges{platform: "windows", beforeSigning: func(files map[string]releaseFixtureFile) {
		file := files["SBOM.cdx.json"]
		file.contents = bytes.ReplaceAll(file.contents, []byte("msvc-static"), []byte("musl-static"))
		files["SBOM.cdx.json"] = file
	}})
	assertFixtureRejected(t, fixture, "RELEASE_SBOM_INVALID", "runtime")
}
