package ports

type CustomerReferenceSource interface {
	Reference(customerID string) (string, error)
}
