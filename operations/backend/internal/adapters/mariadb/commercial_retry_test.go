package mariadb

import (
	"context"
	"errors"
	mysqldriver "github.com/go-sql-driver/mysql"
	"testing"
)

func TestCommercialRetriesTransactionReadConflictsWithinABound(t *testing.T) {
	attempts := 0
	err := retryCommercial(context.Background(), func() error {
		attempts++
		if attempts < 3 {
			return &mysqldriver.MySQLError{Number: 1020}
		}
		return nil
	})
	if err != nil || attempts != 3 {
		t.Fatalf("retry result: %d %v", attempts, err)
	}
	attempts = 0
	err = retryCommercial(context.Background(), func() error { attempts++; return &mysqldriver.MySQLError{Number: 1020} })
	if err == nil || attempts != 4 {
		t.Fatalf("unbounded retry: %d %v", attempts, err)
	}
	terminal := errors.New("invalid snapshot")
	attempts = 0
	err = retryCommercial(context.Background(), func() error { attempts++; return terminal })
	if !errors.Is(err, terminal) || attempts != 1 {
		t.Fatalf("invalid snapshot retried: %d %v", attempts, err)
	}
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	if err := retryCommercial(ctx, func() error { return &mysqldriver.MySQLError{Number: 1020} }); !errors.Is(err, context.Canceled) {
		t.Fatalf("cancellation ignored: %v", err)
	}
}
