package cli

import (
	"fmt"
	"io"

	"github.com/iii-hq/workers/mesh/internal/meshnet"
	"github.com/iii-hq/workers/mesh/internal/service"
)

func Status(w io.Writer) error {
	if removedStatus(service.DefaultPaths().State) {
		_, err := fmt.Fprintln(w, "removed from the mesh by its hub; run mesh leave before joining again")
		return err
	}
	text, err := meshnet.Status()
	if err != nil {
		return err
	}
	_, err = io.WriteString(w, text)
	return err
}
