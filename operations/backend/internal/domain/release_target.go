package domain

import "fmt"

type ReleaseTarget struct {
	Platform     string `json:"platform"`
	Architecture string `json:"architecture"`
}

func ReleaseTargets() []ReleaseTarget {
	return []ReleaseTarget{{Platform: "linux", Architecture: "amd64"}, {Platform: "windows", Architecture: "amd64"}}
}

func (target ReleaseTarget) Supported() bool {
	return target.Architecture == "amd64" && (target.Platform == "linux" || target.Platform == "windows")
}

func (target ReleaseTarget) ArtifactName(version string) string {
	return fmt.Sprintf("customer-%s-%s-%s", target.Platform, target.Architecture, version)
}

func (target ReleaseTarget) FileName(version string) string {
	return fmt.Sprintf("aster-team-%s-%s-%s.tar.gz", version, target.Platform, target.Architecture)
}

func (artifact ReleaseTaskArtifact) Target() ReleaseTarget {
	return ReleaseTarget{Platform: artifact.Platform, Architecture: artifact.Architecture}
}

func (artifact ReleaseArtifact) FileName() string {
	return (ReleaseTarget{Platform: artifact.Platform, Architecture: artifact.Architecture}).FileName(artifact.Version)
}
