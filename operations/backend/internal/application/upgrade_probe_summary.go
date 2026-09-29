package application

import (
	"aster.local/team/operations/backend/internal/domain"
	"time"
)

// Failure windows are sampled observations, not inferred exact outage times.
func recordUpgradeSample(task *domain.EnvironmentUpgrade, sample domain.UpgradeProbeSample) {
	index := -1
	for i := range task.Summary {
		if task.Summary[i].Kind == sample.Kind {
			index = i
			break
		}
	}
	if index < 0 {
		task.Summary = append(task.Summary, domain.UpgradeProbeSummary{Kind: sample.Kind})
		index = len(task.Summary) - 1
	}
	item := &task.Summary[index]
	item.Attempts++
	if !sample.OK {
		item.Failures++
		if item.FailureSince.IsZero() {
			item.FailureSince = sample.At
		}
	}
	if !item.FailureSince.IsZero() {
		end := sample.At.Add(time.Duration(sample.DurationMS) * time.Millisecond)
		elapsed := end.Sub(item.FailureSince).Milliseconds()
		if elapsed > item.LongestFailureMS {
			item.LongestFailureMS = elapsed
		}
		if sample.OK {
			item.FailureSince = time.Time{}
			item.LastRecoveryAt = end
		}
	}
}
